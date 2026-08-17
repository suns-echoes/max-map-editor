//! Bake the shipped templates into scenery cut-outs - the offline step that
//! turns `resources/assets/templates/<PACK>/*.json` into a directory of pieces
//! under `resources/assets/scenery/<PACK>/`.
//!
//! Usage:
//! ```text
//! cargo run -p map-core --example bake_scenery              # every pack
//! cargo run -p map-core --example bake_scenery GREEN DESERT # named packs
//! cargo run -p map-core --example bake_scenery --sheets     # + QA images
//! cargo run -p map-core --example bake_scenery --heights    # + a .hgt per piece
//! ```
//!
//! `--heights` writes each piece's relief out as the `<id>.hgt` beside it, so
//! the pack ships a **height map somebody can open and paint on** rather than a
//! number the renderer re-derives every time it builds an atlas. What it writes
//! is the inference (`map_core::height_field`), which for a piece whose rim was
//! traced is the only door that inference has. Only the packs named on the
//! command line get one; the rest keep inferring, which is still a supported way
//! for a piece to live - and a pack that ships `.hgt` files must keep being baked
//! **with** the flag, because a bake without it deletes them again.
//!
//! The art the bake reads is **source, not a shipped resource**: it lives under
//! `private/sources/scenery/<PACK>/` ([`SOURCE_ART`]), outside the tree that goes
//! into a release. Nothing at runtime opens it; what ships is what the bake
//! writes.
//!
//! A piece may carry **traced lines**: `<id>-X.png`, the hand cut with the
//! shape drawn on it. It is read here and nowhere else - what ships is the `.hgt`
//! it produced. Pure red (`255,0,0,255`) is the **crest** and means one of two
//! things depending on what the piece is:
//!
//! * on a **sunken** piece it is the rim of the ring, drawn as a closed loop,
//!   instead of `map_core`'s one-fraction-for-every-crater guess
//!   (`map_core::rim_dome`);
//! * on a **scarp** it is the brow of the wall, drawn along the inner edge of the
//!   band (`map_core::scarp_rim`).
//!
//! A scarp may also carry pure green (`0,255,0,255`) along the **foot**, the
//! outer edge of the band, and then the wall is authored end to end: ground on
//! the green, peak on the red, and nothing inferred in between. That is what the
//! SNOW cliffs carry. Without it a brow that closes into a loop falls back to the
//! enclosure and one that does not falls back to the light, both of which are
//! guesses the green line makes unnecessary.
//!
//! Either way the piece must be baked **with `--heights`**, because a traced
//! curve is a bake input and nothing at runtime has one: a pack that ships
//! traced pieces and is baked without the flag loses them.
//!
//! Each pack carries a hand-editable `tune.json` beside its output: the shadow
//! ink set, the flat shadow alpha, and the per-object seal radius. The first
//! run writes a starter file from [`ShadowFit::propose`] and says so; after
//! that the file is authoritative and the bake never overwrites it, because the
//! proposal cannot be trusted on a near-neutral ground (SNOW) and because the
//! seal radius is a per-object judgement no statistic makes.
//!
//! `--sheets` writes `temp/scenery/<PACK>/<id>.png`: every cut-out over its own
//! ground, over a foreign ground, and over a checker. The foreign panel is the
//! one that matters - it is where leftover ground shows up as the wrong colour.

use std::path::{Path, PathBuf};

use map_core::{
	CutOpts, GroundInk, PASS_EMPTY, SceneryPack, SceneryPiece, ShadowFit, ShadowInk, Sprite, Template, TileKind,
	TilePack, Transform, apply_game_statics, cut, transform_tile,
};
use max_assets::wrl::TILE_DATA_SIZE;

/// Where the hand-authored art lives - the cut-outs and the traced lines, one
/// directory per pack. Bake input only, and deliberately outside `resources/`:
/// it is worked on by hand and never shipped.
const SOURCE_ART: &str = "private/sources/scenery";

/// The art directory for one pack.
fn source_art(pack: &str) -> PathBuf {
	Path::new(SOURCE_ART).join(pack)
}

const PACKS: [&str; 5] = ["CRATER", "DESERT", "GREEN", "SNOW", "SNOW_DARK"];
const TILE: usize = 64;

/// A candidate shadow ink is auto-accepted into a starter `tune.json` when it
/// is this close to `ground * scale`. Deliberately tight - a false positive
/// punches holes in an object, and the file is meant to be widened by hand.
const ACCEPT_RESIDUAL: f64 = 4.5;
/// ...and only when it darkens the ground without erasing it. Below the floor
/// the ink is not a cast shadow but the art's own black: an outline, a crevice,
/// the inside of a canopy. Pure black fits *every* ground tone (it is `mean *
/// 0`), so without a floor the proposal accepts index 207 in every pack and
/// makes each object's outlines see-through.
const ACCEPT_SCALE: std::ops::RangeInclusive<f64> = 0.35..=0.95;
/// ...and only when it is used enough to be a deliberate ink, not a stray.
const ACCEPT_SHARE: f64 = 0.001;

fn main() {
	let (mut sheets, mut heights) = (false, false);
	let mut wanted: Vec<String> = Vec::new();
	for arg in std::env::args().skip(1) {
		match arg.as_str() {
			"--sheets" => sheets = true,
			"--heights" => heights = true,
			other => wanted.push(other.to_string()),
		}
	}
	let packs: Vec<String> = if wanted.is_empty() { PACKS.iter().map(|p| p.to_string()).collect() } else { wanted };

	let assets = Path::new("resources/assets");
	if !assets.join("tilepacks").is_dir() {
		eprintln!("run from the repo root (no resources/assets/tilepacks here)");
		std::process::exit(2);
	}
	let mut failed = false;
	for pack in &packs {
		if let Err(e) = bake_pack(assets, pack, sheets, heights) {
			eprintln!("{pack}: {e}");
			failed = true;
		}
	}
	if failed {
		std::process::exit(1);
	}
}

fn bake_pack(assets: &Path, name: &str, sheets: bool, heights: bool) -> Result<(), String> {
	let pack = TilePack::load(&assets.join("tilepacks"), name)?;
	let mut palette = pack.palette.clone().ok_or("pack owns no palette")?;
	apply_game_statics(&mut palette);
	let ground = GroundInk::of_pack(&pack);
	if ground.is_empty() {
		return Err("no plain-ground family (LAND with variants) to derive the ground ink from".into());
	}

	let tune_path = assets.join(map_core::SCENERY_DIR).join(name).join("tune.json");
	let tune = match Tune::load(&tune_path)? {
		Some(tune) => tune,
		None => {
			let tune = Tune::propose(&pack, &ground, &palette);
			tune.save(&tune_path)?;
			println!("{name}: wrote a starter {} - review it, then re-run", tune_path.display());
			tune
		}
	};

	let mut pieces = Vec::new();
	// One per piece, in step - the traced lines, or empty where nobody traced.
	let mut rims: Vec<Traced> = Vec::new();
	let dir = assets.join("templates").join(name);
	let art = source_art(name);
	let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
		.map_err(|e| format!("read {}: {e}", dir.display()))?
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.collect();
	files.sort();

	let backdrops = if sheets { Some(ground_backdrop(assets, name)?) } else { None };
	let (mut skipped, mut total_px, mut covered_px, mut hand) = (0usize, 0usize, 0usize, 0usize);
	// Hand-cut pieces whose PNG painted no shadow at all. Worth saying out loud:
	// the alpha rule is the only shadow source a hand-cut piece has, so these
	// objects will stand on the map casting nothing until the art gets a
	// half-alpha shadow drawn under it.
	let mut shadowless: Vec<String> = Vec::new();
	for path in &files {
		let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
		let template = Template::load(path)?;
		let Some((src, pass, obstruction)) = compose(&template, &pack) else {
			println!("  skip {id}: no cell resolves to a tile of this pack");
			skipped += 1;
			continue;
		};
		let (w, h) = (template.width as usize * TILE, template.height as usize * TILE);
		let opts = CutOpts { close: tune.close_of(&id), alpha: tune.shadow_alpha };
		let scarp = tune.piece(&id).and_then(|t| t.scarp).unwrap_or(false);
		let (sprite, rim) = match hand_cut(&art, &id, &src, w, h, &palette, tune.shadow_alpha, scarp)? {
			Some((sprite, rim)) => {
				hand += 1;
				if sprite.shade.iter().all(|&a| a == 0) {
					shadowless.push(id.clone());
				}
				(sprite, rim)
			}
			// Nothing hand-cut, so the flood decides - and the flood needs an
			// object to find. A template of nothing but ground has none (compose).
			None if !obstruction => {
				println!("  skip {id}: no obstruction tile and no hand cut - this is ground, not an object");
				skipped += 1;
				continue;
			}
			None => {
				// The curve is traced on the hand cut, so there is nothing to line
				// it up with here - and a marker left beside a piece nobody cut is
				// a file whose whole purpose is being silently ignored.
				if art.join(format!("{id}{MARKER}")).is_file() {
					return Err(format!("{id}{MARKER}: a rim curve needs the hand-cut {id}.png it was traced on"));
				}
				(cut(&src, w, h, &ground, &tune.shadow, &opts), Traced { crest: Vec::new(), foot: Vec::new() })
			}
		};
		if sprite.is_empty() {
			// Nothing but ground - a meadow or dune template, not an object.
			println!("  skip {id}: cuts to nothing (all ground)");
			skipped += 1;
			continue;
		}
		total_px += w * h;
		covered_px += sprite.covered();
		rims.push(rim);
		if let Some(bg) = &backdrops {
			write_sheet(name, &id, &sprite, &palette, bg)?;
		}
		// Which shape this object stands in, where somebody judged the art and
		// wrote it down - see [`PieceTune`]. Absent for all but a handful, and
		// absent means the family decides, which is how every pack read before
		// `tune.json` could say otherwise.
		let shape = tune.piece(&id);
		pieces.push(SceneryPiece {
			id,
			family: map_core::piece_family(&template.name),
			// Prefilled, not derived: the manifest is what the editor reads, and
			// both fields are meant to be edited there.
			transformable: map_core::Transformable::No,
			peak: None,
			sunken: shape.and_then(|t| t.sunken),
			scarp: shape.and_then(|t| t.scarp),
			height: None,
			name: template.name.clone(),
			cells_w: template.width,
			cells_h: template.height,
			pass,
			sprite,
			user: false,
		});
	}

	// The relief, written as data rather than left to be re-derived. It is the
	// inference either way; what the file buys is that a person can open it - and
	// for a piece whose rim was traced, it is the only door that inference has.
	let (mut drawn, mut traced) = (0usize, 0usize);
	if heights {
		let brightness = map_core::brightness_table(&palette);
		for (piece, rim) in pieces.iter_mut().zip(&rims) {
			let opts = map_core::HeightOpts { rim: &rim.crest, foot: &rim.foot, ..piece.height_opts() };
			piece.height = Some(piece.sprite.height_field(&brightness, &opts));
			drawn += 1;
			traced += usize::from(rim.crest.iter().any(|&c| c));
		}
	}

	let scenery = SceneryPack { pack: name.to_string(), pieces };
	scenery.save(assets)?;
	// The library is a directory of pieces now, so its size is what it takes up
	// on disk rather than one manifest plus one blob.
	let dir = assets.join(map_core::SCENERY_DIR).join(name);
	let bytes: u64 = std::fs::read_dir(&dir)
		.map_err(|e| format!("read {}: {e}", dir.display()))?
		.filter_map(|e| e.ok())
		.filter_map(|e| e.metadata().ok())
		.map(|m| m.len())
		.sum();
	let relief = match (drawn, traced) {
		(0, _) => String::new(),
		(drawn, 0) => format!(", {drawn} height map(s)"),
		(drawn, traced) => format!(", {drawn} height map(s), {traced} off a traced rim"),
	};
	println!(
		"{name}: {} objects ({skipped} skipped, {hand} hand-cut{relief}), {:.1}% of the source box covered, {} on disk",
		scenery.pieces.len(),
		100.0 * covered_px as f64 / total_px.max(1) as f64,
		human(bytes as usize),
	);
	if !shadowless.is_empty() {
		println!("  note: {} hand-cut piece(s) carry no shadow - their PNG paints none:", shadowless.len());
		println!("        {}", shadowless.join(" "));
	}
	Ok(())
}

fn human(bytes: usize) -> String {
	if bytes >= 1 << 20 {
		format!("{:.1} MiB", bytes as f64 / (1 << 20) as f64)
	} else {
		format!("{} KiB", bytes >> 10)
	}
}

/// Flatten a template's **ground-layer** tiles into one box of palette indices,
/// plus the per-cell pass values. `None` = a cell the template left empty. The
/// water layer is what the object was drawn *over*, not part of it, so any id
/// that is not this pack's is dropped.
///
/// `None` when no cell resolves. `obstruction` says whether any cell is one -
/// **the flood needs it and a hand cut does not**, which is why it is reported
/// rather than decided here.
///
/// A template of nothing but `LAND` tiles is ground as far as [`cut`] is
/// concerned: there is no object in it to separate from its background, and
/// flooding one yields a speckle of the few inks the plain-ground families
/// happen not to use - a ground-darkening decal at best, and noise in a picker
/// of objects. That is what the meadow / rough / dune templates are.
///
/// It is *not* what they are once somebody has cut one by hand. The alpha
/// channel is a decision about which pixels are the object, and it outranks a
/// tile flag that was only ever a proxy for the same question - a rough patch
/// somebody cut out of the ground is a piece of scenery whatever its tiles are
/// marked. So the caller skips on this only when there is no hand cut to fall
/// back on.
fn compose(template: &Template, pack: &TilePack) -> Option<(Vec<Option<u8>>, Vec<u8>, bool)> {
	let (w, h) = (template.width as usize * TILE, template.height as usize * TILE);
	let mut src = vec![None; w * h];
	let mut pass = vec![PASS_EMPTY; template.width as usize * template.height as usize];
	let mut any = false;
	let mut obstruction = false;
	for cy in 0..template.height as usize {
		for cx in 0..template.width as usize {
			let spec = &template.cells[cy * template.width as usize + cx];
			for part in spec.split(',').filter(|p| !p.is_empty()) {
				let (id, transform) = match part.split_once(':') {
					Some((id, t)) => (id, Transform::parse(t).unwrap_or_default()),
					None => (part, Transform::default()),
				};
				let Some(&tile) = pack.index_of.get(id) else { continue };
				let mut pixels = [0u8; TILE_DATA_SIZE];
				pixels.copy_from_slice(pack.tile_pixels(tile));
				let pixels = transform_tile(&pixels, transform);
				for y in 0..TILE {
					for x in 0..TILE {
						src[(cy * TILE + y) * w + cx * TILE + x] = Some(pixels[y * TILE + x]);
					}
				}
				pass[cy * template.width as usize + cx] =
					pack.pass.as_ref().and_then(|p| p.get(tile as usize).copied()).unwrap_or(PASS_EMPTY);
				obstruction |= pack.tile_props(tile).is_some_and(|p| p.kind == Some(TileKind::Obstruction));
				any = true;
			}
		}
	}
	any.then_some((src, pass, obstruction))
}

// ----- hand-authored cuts -----------------------------------------------------

/// One object cut straight out of `private/sources/scenery/<PACK>/<id>.png` -
/// `None`
/// when that PNG is the untouched export (`export_template_pngs`), which has no
/// judgement in it.
///
/// The export renders the whole template box and leaves only its *empty cells*
/// transparent. So a PNG that erases any pixel a tile painted - by trimming the
/// image, by rubbing out the ground, or both - is a cut somebody made by hand,
/// and it wins over [`cut`]'s flood.
///
/// **The image is the art**, not a mask over the template: `map_core::cut_image`
/// reads it by the one alpha rule (`map_core::ImageBand`) - transparent is
/// nothing, half-alpha is the cast shadow, everything else is the object's own
/// ink at the nearest palette colour. The pack's `tune.json` shadow ink set has
/// no say here: a dark pixel is dark paint unless the artist made it
/// half-transparent, which is the whole point of cutting by hand.
///
/// Its **`shadowAlpha` does** have a say, though, and the shade plane is stamped
/// with it. The alpha rule is a switch and not an opacity - the artist says
/// *where* the cast shadow is, and `map_core::cut_image` writes the neutral
/// `SHADOW_ALPHA` for want of anything better - but how dark a cast shadow is in
/// this pack is one number the pack already owns, the very one the flood path
/// takes through `CutOpts`. GREEN tunes it to 146 because that is the alpha whose
/// `ShadeTable` lands its grass on ink 238, the shadow ink the game's own art
/// uses; left at 128 a hand-cut GREEN object shaded grass onto 87 and cast a
/// visibly lighter shadow than everything around it.
///
/// The returned sprite's origin is in **template-box** coordinates, so the
/// footprint the manifest records still says where the object sat in its cells.
///
/// The PNG carries no offset, so its place in the box is **found**: the artist
/// only removed pixels, so the image has to sit at the one offset where every
/// pixel it still paints solid is exactly the pixel the box painted there. An
/// ambiguous or impossible fit is an error rather than a guess - a repainted
/// pixel means the PNG is no longer a cut of this template, and silently baking
/// it at the wrong offset would put the object's shadow in the wrong place
/// forever. (So recolouring a body pixel is not a supported edit: trim it, rub
/// it out, or drop it to half alpha - all three keep the fit findable.)
fn hand_cut(
	art: &Path,
	id: &str,
	src: &[Option<u8>],
	w: usize,
	h: usize,
	palette: &[u8],
	alpha: u8,
	scarp: bool,
) -> Result<Option<(Sprite, Traced)>, String> {
	let path = art.join(format!("{id}.png"));
	let Some((rgba, pw, ph)) = read_png(&path)? else { return Ok(None) };
	if pw > w || ph > h {
		return Err(format!("{}: {pw}x{ph} is bigger than the template's {w}x{h} box", path.display()));
	}

	// Body pixels anchor the fit: a shadow pixel's colour is the artist's own
	// paint and need not be anything the template ever held.
	let band = |x: usize, y: usize| map_core::band_of(rgba[(y * pw + x) * 4 + 3]);
	// Every body pixel this offset gets wrong. Counting rather than stopping at
	// the first tells a near miss - a few repainted pixels somebody can go and
	// fix - from a wrong picture entirely.
	let misfits = |ox: usize, oy: usize, want: usize| {
		let mut out: Vec<Misfit> = Vec::new();
		for y in 0..ph {
			for x in 0..pw {
				if band(x, y) != map_core::ImageBand::Body {
					continue;
				}
				let px = (y * pw + x) * 4;
				let painted = [rgba[px], rgba[px + 1], rgba[px + 2]];
				let held = src[(y + oy) * w + x + ox].map(|index| {
					let at = index as usize * 3;
					[palette[at], palette[at + 1], palette[at + 2]]
				});
				if held != Some(painted) {
					out.push(Misfit { x, y, painted, held });
					if out.len() >= want {
						return out;
					}
				}
			}
		}
		out
	};
	let mut hits = Vec::new();
	let mut closest: Option<(usize, (usize, usize))> = None;
	for oy in 0..=h - ph {
		for ox in 0..=w - pw {
			// One misfit is enough to reject; the count only matters for the offset
			// that came nearest, and only to say so in the error.
			let wrong = misfits(ox, oy, ph * pw).len();
			if wrong == 0 {
				hits.push((ox, oy));
			}
			if closest.is_none_or(|(n, _)| wrong < n) {
				closest = Some((wrong, (ox, oy)));
			}
		}
	}
	let (ox, oy) = match hits.as_slice() {
		[one] => *one,
		[] => {
			let (wrong, at) = closest.unwrap_or((0, (0, 0)));
			let show = |m: &Misfit| {
				let hex = |c: &[u8; 3]| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
				let (x, y) = (m.x, m.y);
				match &m.held {
					Some(held) => format!("({x},{y}) paints {} over the template's {}", hex(&m.painted), hex(held)),
					None => format!("({x},{y}) paints {} where the template left the cell empty", hex(&m.painted)),
				}
			};
			let samples: Vec<String> = misfits(at.0, at.1, 4).iter().map(show).collect();
			return Err(format!(
				"{}: no offset in the {w}x{h} box reproduces it - the PNG paints pixels the template does not, so it \
				 is not a cut of this template. Nearest is +{},{} with {wrong} body pixel(s) wrong: {}",
				path.display(),
				at.0,
				at.1,
				samples.join("; ")
			));
		}
		many => {
			return Err(format!(
				"{}: fits the box at {} different offsets {:?} - too little of it is distinctive to place it",
				path.display(),
				many.len(),
				&many[..many.len().min(4)]
			));
		}
	};

	// Is there judgement in this PNG at all? Either it dropped a pixel the
	// template painted, or it painted a shadow - both are decisions the flood
	// cannot make. An untouched export did neither, and goes back to `cut`.
	let mut kept = vec![false; w * h];
	let mut shadowed = false;
	for y in 0..ph {
		for x in 0..pw {
			match band(x, y) {
				map_core::ImageBand::Clear => {}
				map_core::ImageBand::Shadow => shadowed = true,
				map_core::ImageBand::Body => kept[(y + oy) * w + x + ox] = true,
			}
		}
	}
	let trimmed = (0..w * h).any(|i| src[i].is_some() && !kept[i]);
	if !trimmed && !shadowed {
		return Ok(None);
	}

	// The image is the art. Its sprite crops to the PNG, so move the origin out
	// to where the PNG sits in the template box.
	let mut sprite = map_core::cut_image(&rgba, pw, ph, palette);
	// One darkness for every cast shadow in the pack, whoever cut the object.
	for shade in &mut sprite.shade {
		if *shade != 0 {
			*shade = alpha;
		}
	}
	// While the origin is still the crop *inside the PNG*, which is the frame the
	// rim curve was traced in.
	let rim = rim_curve(art, id, pw, ph, &sprite, scarp)?;
	sprite.origin_x += ox as u16;
	sprite.origin_y += oy as u16;
	Ok(Some((sprite, rim)))
}

/// One body pixel a candidate offset gets wrong: what the PNG paints there, and
/// what the template box holds under it (`None` where the template left the cell
/// empty). Only ever built to explain a failed fit.
struct Misfit {
	x: usize,
	y: usize,
	painted: [u8; 3],
	held: Option<[u8; 3]>,
}

/// What a traced rim crest is called: `<id>-X.png` beside `<id>.png`.
const MARKER: &str = "-X.png";

/// The ink a **crest** is traced in - pure opaque red, which no pack palette
/// holds and no cut-out can be mistaken for. A crater's rim, a cliff's brow.
const MARKER_INK: [u8; 4] = [255, 0, 0, 255];

/// The ink a **foot** is traced in - pure opaque green, on the same terms.
///
/// Only a scarp has one, and only a scarp needs one: a crater's low ground is its
/// own silhouette, but a cliff's band has low ground on one side and a raised
/// shelf on the other, and no measurement of the art tells the two apart as well
/// as a person drawing the line does.
const FOOT_INK: [u8; 4] = [0, 255, 0, 255];

/// The two traced lines a marker may carry, in the sprite's own frame - each
/// empty when it was not drawn.
struct Traced {
	/// The red line: a crater's rim, a cliff's brow.
	crest: Vec<bool>,
	/// The green line: the foot of a wall. Never drawn on a crater.
	foot: Vec<bool>,
}

/// **Where a sunken piece's rim crest actually runs**, per sprite pixel, read off
/// `<id>-X.png` - the hand cut with a closed red line drawn along the top of the
/// rim. Empty when there is no such file, which is every piece that is not a
/// crater.
///
/// `map_core::rim_dome` is what it means: peak on the line, climbing to it from
/// ground at the silhouette outside, falling away into the bowl inside. It
/// replaces `RIM_AT`, one fraction eyeballed for the whole pack - a real crater's
/// ring is not one radius, its ejecta reaches further downwind, and two of these
/// are barely round.
///
/// The marker is **the art plus a line**: same size as `<id>.png`, positioned by
/// the same fit, and every red pixel must land on the object. Anything else is an
/// error rather than a silently mis-aligned rim - this is authored data, and the
/// wrong rim is worse than no rim. It is a **bake input**, like the templates:
/// what ships is the `.hgt` it produced.
fn rim_curve(art: &Path, id: &str, pw: usize, ph: usize, sprite: &Sprite, scarp: bool) -> Result<Traced, String> {
	let blank = || Traced { crest: Vec::new(), foot: Vec::new() };
	let path = art.join(format!("{id}{MARKER}"));
	let Some((rgba, mw, mh)) = read_png(&path)? else { return Ok(blank()) };
	if (mw, mh) != (pw, ph) {
		return Err(format!("{}: {mw}x{mh} is not the {pw}x{ph} of the {id}.png it is traced on", path.display()));
	}
	let (sw, sh) = (sprite.width as usize, sprite.height as usize);
	let (cx, cy) = (sprite.origin_x as usize, sprite.origin_y as usize);
	let mut rim = vec![false; sw * sh];
	let mut foot = vec![false; sw * sh];
	let mut strays = 0usize;
	for y in 0..mh {
		for x in 0..mw {
			let ink = &rgba[(y * mw + x) * 4..][..4];
			let line = if ink == MARKER_INK {
				&mut rim
			} else if ink == FOOT_INK {
				&mut foot
			} else {
				continue;
			};
			let (lx, ly) = (x.wrapping_sub(cx), y.wrapping_sub(cy));
			// A crater's rim is traced along its own ejecta and lies on the object
			// throughout. **A cliff's lines do not**: the brow has to cross the gaps
			// in a ragged band, and the foot is drawn on the low ground *beyond* the
			// object outright - which is the whole point of it. So on a scarp a line
			// may fall anywhere, on the object or off it.
			match (lx < sw && ly < sh).then(|| ly * sw + lx) {
				Some(i) if scarp || sprite.body[i] != 0 => line[i] = true,
				// Off the sprite's frame altogether. On a crater that is a curve
				// which does not fit its art and the bake refuses it; on a scarp it
				// is the tail of a foot line drawn past the crop, carrying a
				// direction the rest of the same line already carries.
				_ => strays += 1,
			}
		}
	}
	let drawn = rim.iter().chain(&foot).filter(|&&m| m).count();
	if !scarp && strays > 0 {
		return Err(format!("{}: {strays} pixel(s) of the rim are not on the object", path.display()));
	}
	// A few strays past the crop are a hand drawing a line to its natural end. A
	// *lot* of them is a marker that does not belong to this art, and no share of
	// a wrong curve is worth having.
	if strays > drawn / 4 {
		return Err(format!(
			"{}: {strays} of {} traced pixel(s) fall outside the sprite - this curve does not fit its art",
			path.display(),
			strays + drawn
		));
	}
	if strays > 0 {
		println!("  note {id}: {strays} traced pixel(s) past the crop, dropped - the lines run on past the object");
	}
	if rim.iter().all(|&r| !r) {
		return Err(format!("{}: no {MARKER_INK:?} pixel - a crest is traced in pure red", path.display()));
	}
	// Both lines drawn is the whole shape authored, and then neither the
	// enclosure nor the light is consulted - see map_core::scarp_rim. So the
	// closure check below does not apply: a foot line is exactly what makes an
	// open brow a complete answer.
	if foot.iter().any(|&f| f) {
		if !scarp {
			return Err(format!("{}: a {FOOT_INK:?} foot line, but {id} is not a scarp", path.display()));
		}
		return Ok(Traced { crest: rim, foot });
	}
	// A **bowl** has to be inside something, so a crater's rim that does not close
	// is a curve somebody left open and the guess is worth more than half of it.
	// A **brow** is a line along a wall, and it closes into a loop only when the
	// band it follows happens to: `map_core::scarp_rim` takes an open arc, crests
	// the stretch that was drawn, and leaves the rest to the light. Said out loud
	// rather than passed over, because an arc that was *meant* to close is worth
	// knowing about and the bake is the only place that would notice.
	if !map_core::rim_interior(&rim, sw, sh).iter().any(|&i| i) {
		if !scarp {
			return Err(format!(
				"{}: the rim encloses nothing - the curve has to close for a bowl to be inside it",
				path.display()
			));
		}
		println!("  note {id}: the traced brow is an open arc - the light decides the sides it does not reach");
	}
	Ok(Traced { crest: rim, foot })
}

/// Decode an 8-bit RGB/RGBA PNG to tightly-packed RGBA8, or `None` when the file
/// is not there. Deliberately narrow: these are this repo's own exports, re-cut
/// in a paint program, and anything else should say so rather than be guessed at.
///
/// **Palette PNGs come in through `EXPAND`**, which is not a guess: a `PLTE` plus
/// its `tRNS` says exactly what colour and what alpha each index is, so the
/// expansion is the same picture with the indirection taken out. Paint programs
/// save a cut-out indexed whenever its ink count fits, and a bake that stopped on
/// it would be rejecting art it can read perfectly.
fn read_png(path: &Path) -> Result<Option<(Vec<u8>, usize, usize)>, String> {
	if !path.is_file() {
		return Ok(None);
	}
	let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
	let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
	decoder.set_transformations(png::Transformations::EXPAND);
	let mut reader = decoder.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
	let mut buf = vec![0; reader.output_buffer_size().ok_or("png: image too large")?];
	let info = reader.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
	if info.bit_depth != png::BitDepth::Eight {
		return Err(format!("{}: {:?} PNG - re-export as 8-bit", path.display(), info.bit_depth));
	}
	let src = &buf[..info.buffer_size()];
	let rgba = match info.color_type {
		png::ColorType::Rgba => src.to_vec(),
		png::ColorType::Rgb => src.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
		other => return Err(format!("{}: {other:?} PNG - re-export as RGB or RGBA", path.display())),
	};
	Ok(Some((rgba, info.width as usize, info.height as usize)))
}

// ----- tuning -----------------------------------------------------------------

/// The hand-editable bake input for one pack.
struct Tune {
	shadow: ShadowInk,
	shadow_alpha: u8,
	close: u8,
	overrides: Vec<(String, PieceTune)>,
}

/// What `tune.json`'s `pieces` map says about **one** object.
///
/// The seal radius was the first thing that had to be said per piece; the two
/// relief flags are the second, and they are here rather than in the piece's own
/// `<id>.json` for the reason that file cannot hold them: **the bake rewrites it
/// every run**. A judgement about the art - this dune is a hollow, this cliff is
/// a wall - has to live in the one file the bake reads and never writes, or it
/// survives exactly until the next re-cut.
#[derive(Default)]
struct PieceTune {
	/// This object's seal radius, overriding the pack's.
	close: Option<u8>,
	/// This object is a **depression**, whatever its family says.
	///
	/// SNOW's dunes are the case: `dune-6`, `dune-17` and `dune-18` are scoured
	/// hollows and the other fourteen are drifts, which is not something the
	/// family name "dune" can tell you. It is the *shading* that tells you - the
	/// art is lit from one fixed direction, so a drift is bright on the flank
	/// facing the light and a hollow is bright on the flank facing away.
	sunken: Option<bool>,
	/// This object is the **face of a step in the land** (`map_core::scarp_face`)
	/// rather than a rise on it - SNOW's four `cliff-*` pieces.
	scarp: Option<bool>,
}

impl Tune {
	fn piece(&self, id: &str) -> Option<&PieceTune> {
		self.overrides.iter().find(|(k, _)| k == id).map(|(_, v)| v)
	}

	fn close_of(&self, id: &str) -> u8 {
		self.piece(id).and_then(|p| p.close).unwrap_or(self.close)
	}

	/// A starter tune: every candidate that fits the ground tone tightly enough
	/// to be a shadow beyond argument.
	fn propose(pack: &TilePack, ground: &GroundInk, palette: &[u8]) -> Self {
		let fits = ShadowFit::propose(pack, ground, palette);
		let mut shadow = ShadowInk::new();
		let mut alphas = Vec::new();
		for fit in &fits {
			if fit.residual < ACCEPT_RESIDUAL && ACCEPT_SCALE.contains(&fit.scale) && fit.share >= ACCEPT_SHARE {
				shadow.insert(fit.index);
				alphas.push((fit.share, fit.alpha));
			}
		}
		for fit in fits.iter().take(12) {
			println!(
				"    candidate {:>3}  scale {:.3}  residual {:5.1}  share {:5.2}%{}",
				fit.index,
				fit.scale,
				fit.residual,
				100.0 * fit.share,
				if shadow.contains(fit.index) { "  <- accepted" } else { "" }
			);
		}
		// The flat alpha follows the most-used accepted ink - the one the art
		// spends its shadow on - rather than an average over rare deep shades.
		let shadow_alpha =
			alphas.iter().max_by(|a, b| a.0.total_cmp(&b.0)).map(|&(_, a)| a).unwrap_or(CutOpts::default().alpha);
		Self { shadow, shadow_alpha, close: 0, overrides: Vec::new() }
	}

	fn load(path: &Path) -> Result<Option<Self>, String> {
		let Ok(text) = std::fs::read_to_string(path) else { return Ok(None) };
		let root = json::parse(&text)?;
		let mut shadow = ShadowInk::new();
		for v in root.get("shadow").and_then(|v| v.as_array()).unwrap_or(&[]) {
			let index = v.as_f64().ok_or("tune: a shadow entry is not a number")?;
			if !(0.0..=255.0).contains(&index) {
				return Err(format!("tune: shadow index {index} out of range"));
			}
			shadow.insert(index as u8);
		}
		let byte = |key: &str, fallback: u8| -> u8 {
			root.get(key)
				.and_then(|v| v.as_f64())
				.filter(|f| (0.0..=255.0).contains(f))
				.map(|f| f as u8)
				.unwrap_or(fallback)
		};
		let mut overrides = Vec::new();
		if let Some(pieces) = root.get("pieces").and_then(|v| v.as_object()) {
			for (id, entry) in pieces {
				let tune = PieceTune {
					close: entry.get("close").and_then(|v| v.as_f64()).map(|c| c.clamp(0.0, 255.0) as u8),
					sunken: entry.get("sunken").and_then(|v| v.as_bool()),
					scarp: entry.get("scarp").and_then(|v| v.as_bool()),
				};
				overrides.push((id.clone(), tune));
			}
		}
		Ok(Some(Self {
			shadow,
			shadow_alpha: byte("shadowAlpha", CutOpts::default().alpha),
			close: byte("close", 0),
			overrides,
		}))
	}

	fn save(&self, path: &Path) -> Result<(), String> {
		use json::JsonValue as J;
		if let Some(dir) = path.parent() {
			std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
		}
		let pieces: Vec<(String, J)> = self
			.overrides
			.iter()
			.map(|(id, tune)| {
				let mut entry = Vec::new();
				if let Some(close) = tune.close {
					entry.push(("close".to_string(), J::Number(close as f64)));
				}
				if let Some(sunken) = tune.sunken {
					entry.push(("sunken".to_string(), J::Bool(sunken)));
				}
				if let Some(scarp) = tune.scarp {
					entry.push(("scarp".to_string(), J::Bool(scarp)));
				}
				(id.clone(), J::Object(entry))
			})
			.collect();
		let text = J::Object(vec![
			("version".to_string(), J::String("1".to_string())),
			("shadow".to_string(), J::Array(self.shadow.indices().iter().map(|&i| J::Number(i as f64)).collect())),
			("shadowAlpha".to_string(), J::Number(self.shadow_alpha as f64)),
			("close".to_string(), J::Number(self.close as f64)),
			("pieces".to_string(), J::Object(pieces)),
		])
		.to_pretty();
		std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
	}
}

// ----- QA sheets --------------------------------------------------------------

/// A plain-ground tile from this pack and from a different one, so a cut-out
/// can be judged against the ground it came from *and* one it did not.
struct Backdrops {
	own: Vec<u8>,
	own_palette: Vec<u8>,
	foreign: Vec<u8>,
	foreign_palette: Vec<u8>,
}

fn ground_backdrop(assets: &Path, name: &str) -> Result<Backdrops, String> {
	let other = if name == "GREEN" { "DESERT" } else { "GREEN" };
	let one = |pack_name: &str| -> Result<(Vec<u8>, Vec<u8>), String> {
		let pack = TilePack::load(&assets.join("tilepacks"), pack_name)?;
		let mut palette = pack.palette.clone().ok_or("backdrop pack owns no palette")?;
		apply_game_statics(&mut palette);
		for index in 0..pack.tile_count() {
			let Some(props) = pack.tile_props(index) else { continue };
			if props.kind == Some(TileKind::Land) && props.has_variants {
				return Ok((pack.tile_pixels(index).to_vec(), palette));
			}
		}
		Err(format!("{pack_name}: no plain-ground tile"))
	};
	let (own, own_palette) = one(name)?;
	let (foreign, foreign_palette) = one(other)?;
	Ok(Backdrops { own, own_palette, foreign, foreign_palette })
}

fn write_sheet(pack: &str, id: &str, sprite: &Sprite, palette: &[u8], bg: &Backdrops) -> Result<(), String> {
	let (w, h) = (sprite.width as usize, sprite.height as usize);
	const GAP: usize = 4;
	let sheet_w = w * 3 + GAP * 4;
	let sheet_h = h + GAP * 2;
	let mut rgb = vec![0x18u8; sheet_w * sheet_h * 3];
	for panel in 0..3 {
		for y in 0..h {
			for x in 0..w {
				let under = match panel {
					0 => tile_rgb(&bg.own, &bg.own_palette, x, y),
					1 => tile_rgb(&bg.foreign, &bg.foreign_palette, x, y),
					_ if ((x >> 3) + (y >> 3)) % 2 == 0 => [44, 12, 44],
					_ => [96, 24, 96],
				};
				let i = y * w + x;
				let color = if sprite.body[i] != 0 {
					let p = sprite.body[i] as usize * 3;
					[palette[p], palette[p + 1], palette[p + 2]]
				} else if sprite.shade[i] != 0 {
					let keep = 255 - sprite.shade[i] as u32;
					[0, 1, 2].map(|c| ((under[c] as u32 * keep) / 255) as u8)
				} else {
					under
				};
				let at = ((GAP + y) * sheet_w + GAP + panel * (w + GAP) + x) * 3;
				rgb[at..at + 3].copy_from_slice(&color);
			}
		}
	}
	let dir = Path::new("temp/scenery").join(pack);
	std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
	std::fs::write(dir.join(format!("{id}.png")), png(sheet_w, sheet_h, &rgb)).map_err(|e| format!("write sheet: {e}"))
}

fn tile_rgb(tile: &[u8], palette: &[u8], x: usize, y: usize) -> [u8; 3] {
	let p = tile[(y % TILE) * TILE + (x % TILE)] as usize * 3;
	[palette[p], palette[p + 1], palette[p + 2]]
}

/// A minimal RGB8 PNG with stored (uncompressed) deflate blocks - these are
/// throwaway QA images, and this keeps the example dependency-free.
fn png(width: usize, height: usize, rgb: &[u8]) -> Vec<u8> {
	let mut raw = Vec::with_capacity(height * (width * 3 + 1));
	for y in 0..height {
		raw.push(0); // filter: none
		raw.extend_from_slice(&rgb[y * width * 3..(y + 1) * width * 3]);
	}
	let mut z = vec![0x78, 0x01];
	for (i, chunk) in raw.chunks(0xffff).enumerate() {
		let last = (i + 1) * 0xffff >= raw.len();
		z.push(u8::from(last));
		z.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
		z.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
		z.extend_from_slice(chunk);
	}
	let (mut a, mut b) = (1u32, 0u32);
	for &byte in &raw {
		a = (a + byte as u32) % 65521;
		b = (b + a) % 65521;
	}
	z.extend_from_slice(&((b << 16) | a).to_be_bytes());

	let mut ihdr = Vec::new();
	ihdr.extend_from_slice(&(width as u32).to_be_bytes());
	ihdr.extend_from_slice(&(height as u32).to_be_bytes());
	ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolor
	let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
	for (tag, body) in [(b"IHDR", ihdr), (b"IDAT", z), (b"IEND", Vec::new())] {
		out.extend_from_slice(&(body.len() as u32).to_be_bytes());
		let mut tagged = tag.to_vec();
		tagged.extend_from_slice(&body);
		out.extend_from_slice(&tagged);
		out.extend_from_slice(&crc32(&tagged).to_be_bytes());
	}
	out
}

fn crc32(data: &[u8]) -> u32 {
	let mut crc = !0u32;
	for &byte in data {
		crc ^= byte as u32;
		for _ in 0..8 {
			crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
		}
	}
	!crc
}
