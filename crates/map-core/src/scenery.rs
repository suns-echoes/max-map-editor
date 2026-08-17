//! Scenery cut-outs: a shipped template's art, lifted off the ground it was
//! drawn on.
//!
//! The original art bakes every object into the terrain - a mountain, a stand
//! of trees, a pyramid is a run of 64x64 tiles whose spare pixels are just more
//! ground. To let a user drop that object anywhere, the ground has to come out
//! and the drawn shadow has to become translucent, so whatever the object lands
//! on shows through it and takes its shadow.
//!
//! Three inks, decided per palette index:
//!
//! * **ground** - every index the pack's plain-ground families paint with
//!   ([`GroundInk`]). Derived, not authored: a `LAND` family that has variants
//!   is by definition interchangeable ground, so its pixels are the ground's
//!   whole vocabulary.
//! * **shadow** - the few indices that are the ground tone scaled down
//!   ([`ShadowInk`]). [`ShadowFit::propose`] ranks the candidates, but the call
//!   belongs to the pack's tuning file: on a near-neutral ground (SNOW) "rock in
//!   shadow" and "snow in shadow" are the same colour, and only a human can
//!   separate them.
//! * **body** - everything else. The object.
//!
//! Ground the object *encloses* is not ground. A pyramid's lit faces and the
//! snow trapped inside a rock face are painted with ground inks, and punching
//! them out would leave the object see-through. [`cut`] therefore drops only the
//! ground a flood fill reaches from the sprite's edge, and seals the body mask
//! first ([`CutOpts::close`]) so a dithered edge is not a channel into the
//! interior. The right seal is per-object, not global: a DESERT pyramid needs 2
//! and a GREEN mountain needs 0, or the grass between its outcrops is sealed in
//! and travels with it.
//!
//! What comes out is a [`Sprite`]: a `body` plane of palette indices (`0` =
//! nothing there) and a `shade` plane of alphas (`0` = no shadow). The two are
//! mutually exclusive - a pixel is either the object's own ink or ground the
//! object shades. The renderer reads it the way `units.wgsl` already reads a
//! unit: shade first (flat black over whatever is underneath), then body
//! through the working palette.

use std::path::Path;

use crate::pack::{TileKind, TilePack, Transformable};
use crate::palette::slot_rgb;
use crate::project::{ANIMATED_SLOTS, WATER_SLOTS};

/// A palette index counts as ground once it paints at least this share of the
/// plain-ground families' pixels. Below that it is a stray - one pixel of grit
/// an artist dropped in a corner - not part of the ground's vocabulary.
const GROUND_FLOOR: f64 = 0.0005;

/// Which palette indices a pack's plain ground is painted with.
///
/// Built from the families `tiles.props.json` types `LAND` *with* variants -
/// the interchangeable ones the randomizer treats as one pool. Those are the
/// tiles that are nothing but ground, so between them they use every ground
/// tone and nothing else.
#[derive(Clone)]
pub struct GroundInk {
	is_ground: [bool; 256],
}

impl GroundInk {
	/// Derive the ground vocabulary of `pack`. A pack with no plain-ground
	/// family (WATER) yields an empty set - see [`Self::is_empty`].
	pub fn of_pack(pack: &TilePack) -> Self {
		let mut hist = [0u64; 256];
		let mut total = 0u64;
		for index in 0..pack.tile_count() {
			let Some(props) = pack.tile_props(index) else { continue };
			if props.kind != Some(TileKind::Land) || !props.has_variants {
				continue;
			}
			for &p in pack.tile_pixels(index) {
				hist[p as usize] += 1;
				total += 1;
			}
		}
		let mut is_ground = [false; 256];
		if total > 0 {
			for (index, &count) in hist.iter().enumerate() {
				is_ground[index] = count as f64 / total as f64 >= GROUND_FLOOR;
			}
		}
		Self { is_ground }
	}

	pub fn contains(&self, index: u8) -> bool {
		self.is_ground[index as usize]
	}

	pub fn is_empty(&self) -> bool {
		!self.is_ground.iter().any(|&g| g)
	}

	/// The ground indices, ascending.
	pub fn indices(&self) -> Vec<u8> {
		(0..=255u8).filter(|&i| self.contains(i)).collect()
	}

	/// The unweighted mean colour of the ground inks under `palette` - the tone
	/// a shadow ink is a scaled-down copy of.
	pub fn mean(&self, palette: &[u8]) -> [f64; 3] {
		let mut sum = [0f64; 3];
		let mut n = 0f64;
		for index in self.indices() {
			let rgb = slot_rgb(palette, index);
			for c in 0..3 {
				sum[c] += rgb[c] as f64;
			}
			n += 1.0;
		}
		if n == 0.0 { [0.0; 3] } else { [sum[0] / n, sum[1] / n, sum[2] / n] }
	}
}

/// How well one palette index reads as "the ground, darkened" - the ranking
/// [`ShadowFit::propose`] hands a human to confirm.
#[derive(Debug, Clone, Copy)]
pub struct ShadowFit {
	pub index: u8,
	/// The fraction of the ground tone the ink keeps: `0.65` = 35% darker, so
	/// the ink is black at alpha 0.35 over that ground.
	pub scale: f64,
	/// RMS distance from `ground_mean * scale`, in palette units (0..255). A
	/// shadow ink sits under ~4; object ink that merely happens to be dark
	/// scores worse - except on a near-neutral ground, where everything dark
	/// fits and the ranking says nothing.
	pub residual: f64,
	/// Share of the pack's obstruction pixels painted with this ink.
	pub share: f64,
	/// The alpha that reproduces this ink over the ground's mean tone - in
	/// **linear** space, which is what [`CutOpts::alpha`] and [`ShadeTable`]
	/// speak. It is not `1 - scale`: the fit is measured in the 8-bit space the
	/// art was authored in, and a shadow is applied in the linear space a GPU
	/// blends in. Over the tones a shadow actually falls on, the two land on the
	/// same palette ink; the number differs.
	pub alpha: u8,
}

impl ShadowFit {
	/// Rank every index the pack's obstruction art uses but its plain ground
	/// does not, best fit first. Never authoritative on its own.
	pub fn propose(pack: &TilePack, ground: &GroundInk, palette: &[u8]) -> Vec<ShadowFit> {
		let mut hist = [0u64; 256];
		let mut total = 0u64;
		for index in 0..pack.tile_count() {
			let Some(props) = pack.tile_props(index) else { continue };
			if props.kind != Some(TileKind::Obstruction) {
				continue;
			}
			for &p in pack.tile_pixels(index) {
				hist[p as usize] += 1;
				total += 1;
			}
		}
		let mean = ground.mean(palette);
		let denom: f64 = mean.iter().map(|m| m * m).sum();
		if total == 0 || denom == 0.0 {
			return Vec::new();
		}
		let mut fits = Vec::new();
		for index in 0..=255u8 {
			if hist[index as usize] == 0 || ground.contains(index) || index == 0 {
				continue;
			}
			let rgb = slot_rgb(palette, index);
			// Least-squares scale of `mean` onto `rgb`, then how far off it lands.
			let scale: f64 = (0..3).map(|c| rgb[c] as f64 * mean[c]).sum::<f64>() / denom;
			let residual = ((0..3).map(|c| (rgb[c] as f64 - scale * mean[c]).powi(2)).sum::<f64>() / 3.0).sqrt();
			let alpha = linear_alpha_for(mean, [0, 1, 2].map(|c| (mean[c] * scale).clamp(0.0, 255.0)));
			fits.push(ShadowFit { index, scale, residual, share: hist[index as usize] as f64 / total as f64, alpha });
		}
		fits.sort_by(|a, b| a.residual.total_cmp(&b.residual));
		fits
	}
}

/// The palette indices that shade the ground rather than cover it.
///
/// Membership only - what a shadow pixel *becomes* is [`CutOpts::alpha`], one
/// flat value for the whole object. The per-index alpha the art implies is
/// [`ShadowFit::alpha`], which is how the set gets chosen, not how it is
/// applied.
#[derive(Clone)]
pub struct ShadowInk {
	is_shadow: [bool; 256],
}

impl Default for ShadowInk {
	fn default() -> Self {
		Self { is_shadow: [false; 256] }
	}
}

impl ShadowInk {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn insert(&mut self, index: u8) {
		self.is_shadow[index as usize] = true;
	}

	pub fn contains(&self, index: u8) -> bool {
		self.is_shadow[index as usize]
	}

	pub fn is_empty(&self) -> bool {
		!self.is_shadow.iter().any(|&s| s)
	}

	/// The shadow inks, ascending.
	pub fn indices(&self) -> Vec<u8> {
		(0..=255u8).filter(|&i| self.contains(i)).collect()
	}
}

/// How to cut one object.
#[derive(Debug, Clone, Copy)]
pub struct CutOpts {
	/// Morphological close, in pixels, applied to the body mask before the
	/// ground flood - it seals gaps up to `2 * close` wide so a dithered edge
	/// is not a channel into the object's interior.
	///
	/// Per object, not per pack: `2` for a solid structure whose own faces are
	/// ground-coloured (a DESERT pyramid), `0` for anything the ground is meant
	/// to show through (a GREEN mountain, a stand of trees).
	pub close: u8,
	/// The one alpha every shadow pixel gets, in **linear** space - the space a
	/// GPU blends in, so the editor's preview and [`ShadeTable`]'s export land
	/// on the same colour. Flat by design: the art's own three-step ramp is
	/// dithered, so a single alpha over the same shape reproduces the falloff.
	pub alpha: u8,
}

impl Default for CutOpts {
	fn default() -> Self {
		// 0.45 is what `units.wgsl` already darkens by under a unit, so scenery
		// and units cast the same shadow.
		Self { close: 0, alpha: 115 }
	}
}

/// One cut object: two same-size planes over a cropped box.
///
/// `body[i] != 0` and `shade[i] != 0` are mutually exclusive - a pixel is the
/// object's own ink or ground the object shades, never both.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Sprite {
	pub width: u16,
	pub height: u16,
	/// Where this sprite's top-left sits inside the box it was cut from, in
	/// pixels. The crop is why an object does not carry its template's empty
	/// margin around.
	pub origin_x: u16,
	pub origin_y: u16,
	/// Palette index per pixel; `0` = nothing here.
	pub body: Vec<u8>,
	/// Shadow alpha per pixel; `0` = no shadow here.
	pub shade: Vec<u8>,
}

impl Sprite {
	pub fn is_empty(&self) -> bool {
		self.width == 0 || self.height == 0
	}

	/// Opaque + shaded pixels, for the bake's coverage report.
	pub fn covered(&self) -> usize {
		self.body.iter().zip(&self.shade).filter(|&(&b, &s)| b != 0 || s != 0).count()
	}

	/// The sprite's **centre of mass**: the mean position of its body pixels,
	/// sprite-local. Where the cursor holds a piece while it is being placed,
	/// because a cut-out is an irregular shape whose bounding box says little
	/// about where it *looks* centred - a mountain range with one long spur
	/// hangs off the box's middle, not off its own.
	///
	/// The bounding-box centre when there is no body at all (a shade-only
	/// sprite), so the answer is always inside the sprite.
	pub fn center_of_mass(&self) -> (i32, i32) {
		let (w, h) = (self.width as usize, self.height as usize);
		let (mut sx, mut sy, mut n) = (0u64, 0u64, 0u64);
		for y in 0..h {
			for x in 0..w {
				if self.body[y * w + x] != 0 {
					sx += x as u64;
					sy += y as u64;
					n += 1;
				}
			}
		}
		match n {
			0 => (w as i32 / 2, h as i32 / 2),
			n => ((sx / n) as i32, (sy / n) as i32),
		}
	}

	/// How deep inside the object each pixel sits: the distance from it to the
	/// nearest pixel that is **not** body - a hole, or the world outside the
	/// sprite - in pixels, saturating at 255. `0` wherever there is no body.
	///
	/// Drives [`blend_keeps`], which needs the silhouette's rim to give way and
	/// the core to hold. A two-pass (3,4) chamfer: it is a whole pixel cheaper
	/// than an exact Euclidean transform and never off by more than ~2% over the
	/// [`BLEND_BAND`] pixels that matter, where the choice is a dither threshold
	/// and not a measurement.
	///
	/// Derived, not stored: it follows from `body`, so a `.scn` written before
	/// blending existed needs no re-bake.
	pub fn edge_distance(&self) -> Vec<u8> {
		edge_distance(&self.body_mask(), self.width as usize, self.height as usize)
	}

	/// Which pixels are the object's own ink - the mask both relief passes work
	/// over, and the one thing a `Sprite` and a source PNG have to agree on.
	pub fn body_mask(&self) -> Vec<bool> {
		self.body.iter().map(|&b| b != 0).collect()
	}

	/// The peak this sprite stands at when nothing authored one: half its
	/// shorter side, in the same pixels [`height_field`](Self::height_field)
	/// measures, floored at 1 and capped at the byte the field is stored in.
	///
	/// A guess, and deliberately a crude one - the art carries no elevation, so
	/// the only thing left to read a landform's size off is how much of the map
	/// it covers. Monotone in the sprite's size, which is what the comparison
	/// [`SceneryBlend::Higher`] makes actually needs; a piece whose art disagrees
	/// says so through [`SceneryPiece::peak`].
	pub fn default_peak(&self) -> u8 {
		default_peak(self.width as usize, self.height as usize)
	}

	/// **How high each pixel of this object stands**, `0..=peak`, and `0`
	/// wherever there is no body.
	///
	/// Inferred, never measured: nothing in the art records elevation, so this
	/// reads the two things that do correlate with it and says so plainly.
	///
	/// * The **dome** - [`edge_distance`](Self::edge_distance) over the deepest
	///   the silhouette gets. A landform is tallest where it is widest, so the
	///   distance transform alone already ranks a mountain's core over its skirt.
	///   For a [`sunken`](HeightOpts::sunken) family it is the same dome upside
	///   down: an impact crater's rim stands and its bowl sinks.
	/// * The **luma** of the body's own ink, blurred over [`LUMA_BLUR`] pixels
	///   because the art is dithered and a raw sample is noise, then stretched
	///   over the range this sprite actually uses. The game's terrain art is lit
	///   from one fixed direction, so a lit face is high ground and a crevice is
	///   low - which is the only signal that survives on CRATER and DESERT, whose
	///   shadows were never cut out of the body at all.
	///
	/// Both are then ramped out over the [`GROUND_SKIRT`]: **the outline stands at
	/// ground level**, because that is where the object stops being one, however
	/// brightly its ink is painted there.
	///
	/// The luma term is **not** flipped for a sunken family: dark is low ground
	/// either way, and it is only the overall shape the sign belongs to.
	///
	/// Derived, exactly as `edge_distance` is - it follows from the body plane,
	/// so a `.scn` written before any of this existed needs no re-bake.
	pub fn height_field(&self, brightness: &[u8; 256], opts: &HeightOpts) -> Vec<u8> {
		let luma: Vec<u8> = self.body.iter().map(|&b| brightness[b as usize]).collect();
		height_field(&self.body_mask(), &luma, self.width as usize, self.height as usize, opts)
	}
}

// ----- how high an object stands ----------------------------------------------

/// **How high each pixel of an object stands**, `0..=peak`, and `0` wherever
/// `mask` says there is no object.
///
/// Over a mask and a per-pixel brightness rather than over a [`Sprite`], because
/// the two doors into this are a baked cut-out (ink indices through the pack
/// palette) and the **source PNG** it was cut from (RGB straight off the image),
/// and an inference the two disagreed about would be worth nothing. `luma` is
/// read only where `mask` is set.
///
/// Inferred, never measured: nothing in the art records elevation, so this reads
/// the two things that do correlate with it and says so plainly.
///
/// * The **dome** - [`edge_distance`] over the deepest the silhouette gets. A
///   landform is tallest where it is widest, so the distance transform alone
///   already ranks a mountain's core over its skirt. A
///   [`sunken`](HeightOpts::sunken) family takes the same measurement through
///   [`rim_profile`] instead: ground at the outline, up to a rim, then down into
///   the hole.
/// * The **luma**, blurred over [`LUMA_BLUR`] pixels because the art is dithered
///   and a raw sample is noise, then stretched over the range this object
///   actually uses. The game's terrain art is lit from one fixed direction, so a
///   lit face is high ground and a crevice is low - which is the only signal
///   that survives on CRATER and DESERT, whose shadows were never cut out of the
///   body at all.
///
/// Both are then ramped out over the [`GROUND_SKIRT`]: **the outline stands at
/// ground level**, because that is where the object stops being one, however
/// brightly its ink is painted there. The dome nearly said so already; the luma
/// flatly did not.
///
/// The luma term is **not** flipped for a sunken family: dark is low ground
/// either way, and it is only the overall shape the sign belongs to.
///
/// Derived, exactly as [`edge_distance`] is - it follows from the body plane, so
/// a `.scn` written before any of this existed needs no re-bake.
///
/// **The full account is DESIGN.md 4.4**: why an object has a relief at all,
/// what each signal is worth, what was tried and thrown away, and the four
/// limits this inference is known to have. `examples/height_pngs` is how to look
/// at the answer.
pub fn height_field(mask: &[bool], luma: &[u8], width: usize, height: usize, opts: &HeightOpts) -> Vec<u8> {
	if width == 0 || height == 0 {
		return Vec::new();
	}
	assert_eq!(mask.len(), width * height, "height_field: mask is not width * height");
	assert_eq!(luma.len(), width * height, "height_field: luma is not width * height");
	let edge = edge_distance(mask, width, height);
	let lit = stretched_luma(mask, luma, width, height);
	let peak = opts.peak.max(1) as u32;
	// A drawn rim replaces the guess at where the ring stands - see [`rim_dome`].
	let drawn = opts.sunken.then(|| rim_dome(mask, opts.rim, &edge, width, height)).flatten();
	// ...and a pyramid is not guessed at either - see [`box_distance`].
	let built = (opts.pyramid && !opts.sunken).then(|| box_distance(width, height));
	// ...and a scarp is a step in the land rather than a rise on it - see
	// [`scarp_face`]. One shape to a piece: a wall is not also a bowl or a tomb.
	// ...and a wall whose brow somebody traced is not guessed at at all - see
	// [`scarp_rim`], which the light-reading [`scarp_face`] only backs up.
	let faced = (opts.scarp && !opts.sunken && !opts.pyramid)
		.then(|| {
			let face = scarp_face(mask, width, height);
			scarp_rim(mask, opts.rim, opts.foot, face.as_ref(), width, height).or(face)
		})
		.flatten();
	// **How far in a pixel is, and from what.** The silhouette for anything the
	// art was read for; the sprite's own box for a built pyramid, whose base is
	// the footprint and whose causeway is a gap in its ink rather than an outline;
	// the distance up from the foot for a scarp, whose brow is not ground at all.
	// The dome, the skirt and how wide the skirt can be all measure with this one
	// ruler, or they would disagree about where the object stops.
	let inward: Vec<u32> = edge.iter().map(|&d| d as u32).collect();
	let depth: &[u32] = match (&built, &faced) {
		(Some(built), _) => built,
		(_, Some(faced)) => &faced.foot,
		_ => &inward,
	};
	let deepest = depth.iter().copied().max().unwrap_or(0).max(1);
	// The skirt this object can afford - see [`GROUND_SKIRT`]. Never the whole
	// depth: a ribbon that is edge all the way through keeps the relief it has
	// rather than being sanded flat.
	let skirt = GROUND_SKIRT.min(deepest.saturating_sub(1));
	let mut out = vec![0u8; width * height];
	for i in 0..width * height {
		if !mask[i] {
			continue;
		}
		// **A known shape takes the luma's say away from it.** For a drawn rim
		// ([`RimDome`]) that is the full weight out in the ejecta, none of it on
		// the crest or inside the bowl, ramped between over [`CREST_FADE`]; for a
		// pyramid it is none of it anywhere. Nothing inferred overrules a shape
		// somebody drew or built, and the light on a pyramid's two faces would
		// tilt it towards the sun.
		let (dome, luma_share) = match (&drawn, &built, &faced) {
			(Some(drawn), _, _) => (drawn.dome[i], drawn.luma[i]),
			// Four flat faces meeting at an apex: the depth over the deepest it
			// gets *is* the pyramid, and neither inferred signal is consulted.
			(_, Some(_), _) => ((depth[i] * 255 / deepest).min(255), 0),
			// An inferred wall keeps its luma: the shape says which way the land
			// steps and the light says how the face is modelled, unlike a pyramid
			// whose two flat planes stand at the same height however lit. A drawn
			// brow takes that say away over the crest, as a drawn rim does.
			(_, _, Some(faced)) => (faced.dome[i], faced.luma[i]),
			_ => {
				let dome = (depth[i] * 255 / deepest).min(255);
				(if opts.sunken { rim_profile(dome) } else { dome }, LUMA_WEIGHT)
			}
		};
		let total = DOME_WEIGHT + LUMA_WEIGHT;
		let relief = (dome * (total - luma_share) + lit[i] as u32 * luma_share) / total;
		// The outline meets the ground: an object's own edge is where it stops
		// being an object, whatever its ink does there.
		let relief = match skirt {
			0 => relief,
			s => relief * depth[i].saturating_sub(1).min(s) / s,
		};
		// A body pixel never reads as 0: that is the height of bare ground, and
		// an object flush with the ground is not what "no object" means.
		out[i] = ((relief * peak + 127) / 255).clamp(1, peak) as u8;
	}
	out
}

/// **A pyramid, built rather than inferred** - how far each pixel is from the
/// nearest edge of the sprite's own box, in pixels. Normalized in
/// [`height_field`] it is four flat triangular faces meeting at an apex, or at a
/// short ridge where the box is not square, which is what a rectangular pyramid
/// has.
///
/// Every other shape here is read off the art because nothing in the art records
/// elevation. A pyramid is the one landform whose shape is **known before you
/// look**: it is a pyramid. So it is constructed from the footprint, and the two
/// inferred signals are not consulted at all -
///
/// * the **dome** would be near enough right on a plain block, but DESERT's four
///   pyramids are quartered by a causeway of half-alpha steps, which is shadow
///   and not body. The silhouette is therefore four separate islands, and a
///   distance transform reads it as four little hills with a valley crossing
///   between them - which is what it was doing.
/// * the **luma** would tilt the whole thing towards the sun. A pyramid's faces
///   are flat planes at equal slope; the lit one and the dark one stand exactly
///   as high as each other, and this is the one object where reading brightness
///   as height is *guaranteed* to be wrong rather than merely unreliable.
///
/// Over the box rather than over the body, so the causeway does not divide it
/// and a ragged base course does not dent the faces. [`GROUND_SKIRT`] measures
/// with this same ruler for a pyramid: the base of the built shape is the box's
/// own edge, and skirting off the silhouette instead would sink every pixel
/// beside the causeway to the ground it is not next to.
fn box_distance(width: usize, height: usize) -> Vec<u32> {
	(0..width * height)
		.map(|i| {
			let (x, y) = (i % width, i / width);
			x.min(y).min(width - 1 - x).min(height - 1 - y) as u32
		})
		.collect()
}

/// Whether a family's pieces are pyramids, whose shape is known rather than
/// inferred ([`pyramid_dome`]).
///
/// By name, exactly as [`family_is_sunken`] is, and for the same reason: the
/// name is the only thing a cut-out carries that says what it *is*.
pub fn family_is_pyramid(family: &str) -> bool {
	family == "pyramid"
}

/// **The step from a pixel towards the sun**, in whole pixels of the sprite's
/// own grid.
///
/// The game's terrain art is lit from one fixed direction - the fact the luma
/// term already rests on ([`height_field`]) - and this is that direction written
/// down, for the one shape that needs to know which *way* it points rather than
/// merely that it exists. Up and left: measured off the drawn shadows the cut
/// separates out of the body, whose centroid sits south-east of the body's on
/// every SNOW piece that paints one (`temp/shade-side.mjs`, and the four cliffs
/// agree with the mountains).
///
/// A diagonal step rather than a normalized vector because a walk over a mask
/// takes whole pixels, and 45 degrees is what the art draws.
const LIGHT_STEP: (isize, isize) = (-1, -1);

/// Whether a family's pieces are **scarps** - the edge of a plateau rather than
/// a ridge standing on it ([`scarp_face`]).
///
/// By name, exactly as [`family_is_sunken`] and [`family_is_pyramid`] are. No
/// shipped family is one by default: SNOW's `cliff-*` pieces are the case this
/// exists for and they are marked per piece, because "cliff" names how the art
/// was drawn and not reliably which way the land behind it goes.
pub fn family_is_scarp(_family: &str) -> bool {
	false
}

/// **A scarp: the wall band alone, ground at its foot and peak at its brow.**
///
/// A ridge is high in the middle and falls away on both sides, which is what the
/// dome makes of any band and is what every cut-out here was getting. A cliff is
/// not that shape. It is the *edge of a step in the land*: low ground on one
/// side, high ground on the other, and the band of art is the face between them.
/// Read as a dome, a cliff's brow sinks back to ground level over the last few
/// pixels of the very edge that should be its highest - so an object placed just
/// behind the wall floats over it instead of standing on the shelf.
///
/// **Which side is high is not authored, because the light already says.** A
/// wall throws its cast shadow onto the low ground at its foot; where the high
/// side lies, the shadow falls on ground at its own level and there is nothing
/// to see. So the brow is the [`LIGHT_STEP`] side and the foot is the other, and
/// because that is measured per pixel along the light axis it follows a band
/// around a curve - which a single direction per piece could not, SNOW's
/// `cliff-4` being an S.
///
/// **The two flanks are labelled, and then measured with a distance
/// transform.** The ground outside the wall is split in two - the flank the
/// light comes over is the high side, the flank the wall throws its shadow onto
/// is the low one - and every body pixel is then placed between them by
/// `to_low / (to_low + to_high)`. That is the ruler [`rim_dome`] measures an
/// outer flank with, and for the same reason: each cross-section gets the slope
/// its own width implies, so a wall that thickens does not also steepen.
///
/// **Walking the light axis pixel by pixel was tried first and stripes.** A run
/// length along one diagonal is a 1-D measurement with no lateral smoothing, so
/// a ragged silhouette gives neighbouring diagonals wildly different runs and the
/// relief comes out combed. A chamfer distance to a *region* is 2-D and has no
/// such seam - and it stops caring which way the band happens to run, which the
/// walk very much did.
///
/// **The enclosure was tried first of all and cannot be used**: flooding the
/// transparent field in from the border and calling what it fails to reach the
/// high side works only on a closed loop, and none of the four SNOW cliffs is
/// one. They are open arcs; the flood reaches straight through the gap and
/// leaves a few hundred pixels of speckle against two hundred thousand of body.
/// Sealing the gap first needs 16 px of dilation on three of them and 24 on
/// `cliff-4`, which is a per-piece number nobody can defend.
///
/// **Where the brow runs can be drawn instead**, and then none of the above is
/// guessed at - see [`scarp_rim`]. This is the fallback for a wall nobody traced.
///
/// `None` when the art gives only one flank - a piece with no low side is not a
/// step in the land, and the plain dome is worth more than half a wall.
fn scarp_face(mask: &[bool], width: usize, height: usize) -> Option<Scarp> {
	// **Which flank is which.** Sweeping along the light axis, how far back it is
	// to the last body pixel: a ground pixel the wall stands north-west of is
	// ground the wall shadows, so it is the foot. `None` where the sweep has not
	// passed through the body at all.
	let toward = |step: (isize, isize)| {
		let mut out: Vec<Option<u32>> = vec![None; width * height];
		// Against the step, so the neighbour a pixel reads is already answered.
		let ys: Vec<usize> = if step.1 < 0 { (0..height).collect() } else { (0..height).rev().collect() };
		let xs: Vec<usize> = if step.0 < 0 { (0..width).collect() } else { (0..width).rev().collect() };
		for &y in &ys {
			for &x in &xs {
				let i = y * width + x;
				let (nx, ny) = (x as isize + step.0, y as isize + step.1);
				if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
					continue;
				}
				let n = ny as usize * width + nx as usize;
				out[i] = if mask[n] { Some(0) } else { out[n].map(|d| d + 1) };
			}
		}
		out
	};
	let sunward = toward(LIGHT_STEP);
	let leeward = toward((-LIGHT_STEP.0, -LIGHT_STEP.1));
	// A nook in a ragged silhouette lies both ways from the body at once. The
	// nearer wall is the one it belongs to, which is what a flank means.
	let (mut low, mut high) = (vec![false; width * height], vec![false; width * height]);
	for i in 0..width * height {
		if mask[i] {
			continue;
		}
		match (sunward[i], leeward[i]) {
			(Some(a), Some(b)) if a <= b => low[i] = true,
			(Some(_), Some(_)) => high[i] = true,
			(Some(_), None) => low[i] = true,
			(None, Some(_)) => high[i] = true,
			(None, None) => {}
		}
	}
	if !low.iter().any(|&s| s) || !high.iter().any(|&s| s) {
		return None;
	}
	// Outside seeds, exactly as the silhouette's own [`edge_distance`] takes
	// them: the frame's edge is ground the piece was cropped away from, not a
	// wall it runs into.
	let to_low = chamfer(&low, width, height, true);
	let to_high = chamfer(&high, width, height, true);
	let dome =
		(0..width * height).map(|i| 255 * to_low[i] as u32 / (to_low[i] as u32 + to_high[i] as u32).max(1)).collect();
	let foot: Vec<u32> = to_low.into_iter().map(u32::from).collect();
	Some(Scarp { dome, luma: vec![LUMA_WEIGHT; width * height], foot })
}

/// **Where a wall's brow runs, drawn rather than inferred** - `0..=255` per
/// pixel, `None` when there is no usable curve and [`scarp_face`] should read the
/// light instead.
///
/// The same door a crater's rim has ([`rim_dome`]) and the same file:
/// `private/sources/scenery/<PACK>/<id>-X.png`, the hand cut with the shape drawn on it -
/// pure red on the inner side of the band, pure green on the outer.
///
/// **The lines say which way, not how high.** They are drawn freehand along a
/// band that is ragged on both edges, so reading them as contours - peak exactly
/// on the red, ground exactly on the green - takes a loose gesture for a
/// measurement, and every wobble of the artist's hand becomes a bump in the
/// relief. What a person can say reliably at that scale is the *direction*: the
/// land is higher over there and lower over here. So the lines are used for
/// nothing but labelling the two grounds -
///
/// * every transparent pixel goes to whichever line it is nearer;
/// * the wall then runs from ground at the silhouette it shares with the green
///   side to its peak at the silhouette it shares with the red side, by
///   `to_low / (to_low + to_high)`.
///
/// So the **edges of the band** set the heights, as they do everywhere else in
/// this inference, and the lines only orient it. Moving a line a few pixels does
/// not move the relief at all; moving it across the band flips that stretch of
/// wall, which is the only thing it should be able to do.
///
/// This is [`scarp_face`] with the sides taken from a drawing instead of from the
/// sun, and it is strictly better where it applies: the light is one direction
/// for the whole piece and cannot follow a loop around, which is exactly what a
/// cliff does.
///
/// `None` unless **both** lines are there - one line is half a direction. A red
/// curve alone still means what it always did on a sunken piece ([`rim_dome`]);
/// on a wall it leaves the sides to `fallback` and the light.
fn scarp_rim(
	mask: &[bool],
	rim: &[bool],
	drawn_foot: &[bool],
	fallback: Option<&Scarp>,
	width: usize,
	height: usize,
) -> Option<Scarp> {
	let drawn = |line: &[bool]| line.len() == mask.len() && line.iter().any(|&m| m);
	if !drawn(rim) {
		return None;
	}
	if !drawn(drawn_foot) {
		return scarp_rim_crest(mask, rim, fallback, width, height);
	}
	// **Whichever line a pixel of ground is nearer is the side it is on.** A
	// distance to each, and the comparison is the whole reading - which is what
	// makes a wobble in either line cost nothing: it moves the boundary between
	// the two grounds by half the wobble, out in ground that has no relief.
	let to_red = chamfer(rim, width, height, false);
	let to_green = chamfer(drawn_foot, width, height, false);
	let (mut low, mut high) = (vec![false; mask.len()], vec![false; mask.len()]);
	for i in 0..mask.len() {
		if mask[i] {
			continue;
		}
		match to_green[i] <= to_red[i] {
			true => low[i] = true,
			false => high[i] = true,
		}
	}
	if !low.iter().any(|&s| s) || !high.iter().any(|&s| s) {
		return None;
	}
	// Not outside seeds: both grounds are real pixels inside the frame here, and
	// the frame's edge belongs to whichever of them reaches it.
	let to_low = chamfer(&low, width, height, false);
	let to_high = chamfer(&high, width, height, false);
	let dome =
		(0..mask.len()).map(|i| 255 * to_low[i] as u32 / (to_low[i] as u32 + to_high[i] as u32).max(1)).collect();
	// The luma keeps its say all the way across. It had none on a crest because a
	// crest was authored and the light was not; a direction is not a height, and
	// there is nothing here for it to contradict.
	Some(Scarp { dome, luma: vec![LUMA_WEIGHT; mask.len()], foot: to_low.into_iter().map(u32::from).collect() })
}

/// A wall with a **red line and no green one** - the older reading, kept for a
/// crest traced before the foot line existed.
///
/// Here the red line *is* taken for the crest, because on its own it has nothing
/// else to be: peak on the line and past it, climbing to it from whichever ground
/// the curve's enclosure - or failing that the light - says is the low one. Less
/// forgiving than [`scarp_rim`] proper, which is the reason the green line was
/// worth drawing.
fn scarp_rim_crest(
	mask: &[bool],
	rim: &[bool],
	fallback: Option<&Scarp>,
	width: usize,
	height: usize,
) -> Option<Scarp> {
	let inside = rim_interior(rim, width, height);
	let low: Vec<bool> = (0..mask.len()).map(|i| !mask[i] && !inside[i] && !rim[i]).collect();
	let measured: Vec<u32> = match inside.iter().any(|&i| i) && low.iter().any(|&s| s) {
		true => chamfer(&low, width, height, true).into_iter().map(u32::from).collect(),
		false => fallback?.foot.clone(),
	};
	// **The brow is never sanded down.** [`GROUND_SKIRT`] grounds a piece's own
	// outline, and on the crest and behind it that is the wrong reading twice
	// over: the drawn line is where the wall is highest, and the ground it runs
	// alongside is the shelf it holds up rather than the plain it rises from.
	let foot: Vec<u32> = (0..mask.len())
		.map(|i| match rim[i] || inside[i] {
			true => measured[i].max(GROUND_SKIRT + 1),
			false => measured[i],
		})
		.collect();
	let to_rim = chamfer(rim, width, height, false);
	let dome = (0..mask.len())
		.map(|i| match rim[i] || inside[i] {
			true => 255,
			false => 255 * foot[i] / (foot[i] + to_rim[i] as u32).max(1),
		})
		.collect();
	let luma = (0..mask.len())
		.map(|i| match rim[i] || inside[i] {
			true => 0,
			false => LUMA_WEIGHT * (to_rim[i] as u32).min(CREST_FADE) / CREST_FADE,
		})
		.collect();
	Some(Scarp { dome, luma, foot })
}

/// What [`scarp_face`] makes of a wall: the shape across it, how much of a say
/// the luma keeps, and the ruler its skirt is measured with.
struct Scarp {
	/// `0..=255` per pixel, `0` at the foot and `255` on the brow.
	dome: Vec<u32>,
	/// The luma's share of the answer here, out of `DOME_WEIGHT + LUMA_WEIGHT`.
	/// [`LUMA_WEIGHT`] all the way across an inferred wall, and nothing on a
	/// drawn crest - see [`scarp_rim`].
	luma: Vec<u32>,
	/// How far up from the foot each pixel is - what [`GROUND_SKIRT`] measures
	/// with here.
	///
	/// The silhouette is the wrong ruler for a scarp, exactly as it is for a
	/// pyramid ([`box_distance`]): skirting off the outline would sand the brow
	/// back down to the ground it is the whole point of not being on. Only the
	/// foot is ground level, so only the foot gets a skirt.
	foot: Vec<u32>,
}

/// Where a sunken object's **rim** stands, as a depth into the silhouette in the
/// same `0..=255` the dome is measured in: 35% of the way from the outline to
/// the deepest point inside it.
///
/// Eyeballed against the CRATER pack's `crater-*` art with
/// `examples/height_pngs`, not calculated - the ejecta blanket a crater is cut
/// with reaches well past its rim, so the ring sits a good way in from the
/// silhouette.
const RIM_AT: u32 = 89;

/// The dome a **sunken** object stands in: ground at the outline, rising to the
/// rim at [`RIM_AT`], then falling away inwards to the bowl.
///
/// Not the dome upside down, which was the first shape tried and is wrong in a
/// way you can see: inverting makes the *outermost silhouette pixel* the highest
/// point, so a crater reads as one smooth funnel with its lip at the outer edge
/// of its own ejecta. A crater is a **ring**: its edge is ground level, it rises
/// to a raised rim, and only then does it drop into the hole.
///
/// `depth` and the result are both `0..=255` - `0` at the outline, `255` at the
/// deepest point inside the silhouette.
fn rim_profile(depth: u32) -> u32 {
	match depth {
		d if d <= RIM_AT => d * 255 / RIM_AT,
		d => 255 - (d - RIM_AT) * 255 / (255 - RIM_AT),
	}
}

/// The dome a sunken object stands in when **somebody drew where its rim is** -
/// `0..=255` per pixel, `None` when there is no usable curve and [`rim_profile`]
/// should guess at it instead.
///
/// [`RIM_AT`] is one number for every crater in the pack, and a crater is not a
/// ring of one radius: the ejecta reaches further downwind, the bowl sits off
/// centre, and two of the CRATER pieces are barely round at all. So the rim can
/// be **traced on the art** - `private/sources/scenery/<PACK>/<id>-X.png`, the cut-out
/// with a closed red line along the crest - and this is what that tracing means:
///
/// * **On the line**, the object stands at its peak. That is what was drawn.
/// * **Outside it** (the outer flank, the ejecta blanket) the relief climbs from
///   ground at the silhouette to the peak at the line, as the share of the way
///   across it each pixel has come: `edge / (edge + to_rim)`. Distance-based
///   rather than a fixed fraction, so a lopsided crater's near and far flanks
///   each get the slope their own width implies.
/// * **Inside it** (the bowl) it falls away from the rim **to ground level** at
///   the deepest point in there. [`height_field`] weighs the luma at nothing
///   inside the curve, so this is the whole answer in a bowl and the floor really
///   does reach the ground the clamp puts it at.
///
/// **The light says nothing useful inside a bowl.** Everywhere else the luma is
/// worth 40% - the art is lit from one direction, so a lit face is high ground.
/// Down a hole it is measuring which way the floor tilts, and on `crater-1`,
/// `crater-2` and `crater-8` it is measuring the bright pool of ink at the very
/// bottom: the whitest paint in those three pieces is their *lowest* point, so
/// the correlation the luma rests on is not merely weak in there but inverted.
/// The curve is authored and the light is inferred; where they disagree, the
/// drawing wins outright rather than by six votes to four. So the luma's share
/// is nothing on the line and inside it, fading back to full over
/// [`CREST_FADE`] - and a floor that reaches ground level is what the caller
/// gets, rather than 40% of however brightly it was painted.
///
/// `None` unless the curve is the right size for the mask, marks something, and
/// **encloses something** ([`rim_interior`]). An open curve has no inside, and
/// half a rim is worth less than the guess it would replace - the same rule a
/// mis-sized `.hgt` follows.
fn rim_dome(mask: &[bool], rim: &[bool], edge: &[u8], width: usize, height: usize) -> Option<RimDome> {
	if rim.len() != mask.len() || !rim.iter().any(|&r| r) {
		return None;
	}
	let inside = rim_interior(rim, width, height);
	if !inside.iter().zip(mask).any(|(&i, &m)| i && m) {
		return None;
	}
	let to_rim = chamfer(rim, width, height, false);
	// How far into the bowl it is from the rim to the deepest point in there.
	let bowl = (0..mask.len()).filter(|&i| inside[i] && mask[i]).map(|i| to_rim[i] as u32).max().unwrap_or(0).max(1);
	let mut dome: Vec<u32> = (0..mask.len())
		.map(|i| match (rim[i], inside[i]) {
			(true, _) => 255,
			(_, true) => 255 - (to_rim[i] as u32 * 255 / bowl).min(255),
			(_, false) => {
				let (out, up) = (edge[i] as u32, to_rim[i] as u32);
				255 * out / (out + up).max(1)
			}
		})
		.collect();
	// The bowl is the distance to the rim and nothing else now, so the distance
	// transform's own creases show - see [`BOWL_SMOOTH`]. Rounding the deepest
	// point off lifts it, so the floor is stretched back down afterwards: a bowl
	// bottoms out at ground level, which is the whole shape of the thing, and a
	// blur is not allowed to take that away.
	let smooth = box_mean(&dome, &inside, BOWL_SMOOTH.min(bowl as usize / 2).max(1), width, height);
	let floor = (0..mask.len()).filter(|&i| inside[i] && mask[i]).map(|i| smooth[i]).min().unwrap_or(0);
	for i in 0..mask.len() {
		if inside[i] {
			dome[i] = (smooth[i].saturating_sub(floor)) * 255 / (255 - floor).max(1);
		}
	}
	let luma = (0..mask.len())
		.map(|i| match inside[i] || rim[i] {
			true => 0,
			false => LUMA_WEIGHT * (to_rim[i] as u32).min(CREST_FADE) / CREST_FADE,
		})
		.collect();
	Some(RimDome { dome, luma })
}

/// What a traced rim makes of an object: the shape it stands in, and how much of
/// a say the luma keeps at each pixel - the two things [`height_field`] needs
/// from a drawing.
struct RimDome {
	/// `0..=255` per pixel, peak on the traced line.
	dome: Vec<u32>,
	/// The luma's share of the answer here, out of `DOME_WEIGHT + LUMA_WEIGHT` -
	/// [`LUMA_WEIGHT`] out in the ejecta, nothing on the line and inside it.
	luma: Vec<u32>,
}

/// How far outside a traced rim the luma still has a say, in pixels: full weight
/// this far out, none of it on the line.
///
/// The switch has to be a ramp rather than a step. Drop the luma on the line
/// alone and the crest gains a one-pixel cliff - the pixel outside it is pulled
/// towards its own ink by 40% while the line is not pulled at all, which on dark
/// paint is a tenth of the object's whole height over one pixel, and `Higher`
/// blends along exactly such contours.
///
/// Three pixels, matching [`LUMA_BLUR`], because that is the radius over which
/// the blur has already smeared the crest's own ink across the line: the band
/// where the luma is least entitled to an opinion about which side of the rim a
/// pixel is on.
const CREST_FADE: u32 = 3;

/// How far the bowl's floor is smoothed, in pixels - never more than half its
/// own depth, so a shallow bowl is not flattened by it.
///
/// A distance-to-boundary field is a **cone over its region**, and a cone over a
/// hand-drawn wobble has a medial axis: creases where the nearest point on the
/// curve jumps from one lobe to another. They are real geometry and they are not
/// terrain - a crater floor is a paraboloid, not a folded tent - and everywhere
/// else in this inference the luma is what hides them. Inside a traced rim it is
/// weighed at nothing, so the creases come out as spokes across the floor unless
/// something takes them off.
///
/// Six pixels. Smaller than the smallest bowl and much larger than the kink,
/// which is what a crease needs: it is a break in slope rather than in height,
/// so a blur wide enough to round it is not wide enough to move the floor.
const BOWL_SMOOTH: usize = 6;

/// Which pixels a closed curve **encloses** - everything the outside world
/// cannot reach without crossing `rim`. The curve itself is not interior.
///
/// A 4-connected flood from the border, so an 8-connected line seals it: that is
/// what a paint program's pencil draws, and a curve that leaks is one somebody
/// left open. All-false when nothing is enclosed, which is how the caller tells
/// an open curve from a closed one.
pub fn rim_interior(rim: &[bool], width: usize, height: usize) -> Vec<bool> {
	let n = width * height;
	if rim.len() != n || n == 0 {
		return vec![false; n];
	}
	let border = (0..width)
		.flat_map(|x| [x, (height - 1) * width + x])
		.chain((0..height).flat_map(|y| [y * width, y * width + width - 1]));
	let mut outside = vec![false; n];
	let mut stack: Vec<usize> = Vec::new();
	for i in border {
		if !rim[i] && !outside[i] {
			outside[i] = true;
			stack.push(i);
		}
	}
	while let Some(i) = stack.pop() {
		let (x, y) = (i % width, i / width);
		let sides = [
			(x > 0).then(|| i - 1),
			(x + 1 < width).then(|| i + 1),
			(y > 0).then(|| i - width),
			(y + 1 < height).then(|| i + width),
		];
		for j in sides.into_iter().flatten() {
			if !rim[j] && !outside[j] {
				outside[j] = true;
				stack.push(j);
			}
		}
	}
	(0..n).map(|i| !outside[i] && !rim[i]).collect()
}

/// How deep inside the object each pixel sits: the distance from it to the
/// nearest pixel `mask` does not set - a hole, or the world outside the box - in
/// pixels, saturating at 255. `0` wherever the mask is clear.
///
/// Drives [`blend_keeps`], which needs the silhouette's rim to give way and the
/// core to hold, and the dome term of [`height_field`]. A two-pass (3,4)
/// chamfer: it is a whole pixel cheaper than an exact Euclidean transform and
/// never off by more than ~2% over the [`BLEND_BAND`] pixels that matter, where
/// the choice is a dither threshold and not a measurement.
pub fn edge_distance(mask: &[bool], width: usize, height: usize) -> Vec<u8> {
	// The world outside the box is not the object either, so a border pixel is
	// one step in from it.
	chamfer(&mask.iter().map(|&b| !b).collect::<Vec<bool>>(), width, height, true)
}

/// The distance from every pixel to the nearest `seed`, in pixels, saturating at
/// 255. `0` on a seed. `outside_seeds` says whether the world beyond the box
/// counts as one - true for a silhouette, whose outside genuinely is not the
/// object, and false for a curve drawn inside the frame, which is only where it
/// was drawn.
///
/// A two-pass (3,4) chamfer: it is a whole pixel cheaper than an exact Euclidean
/// transform and never off by more than ~2% over the [`BLEND_BAND`] pixels that
/// matter, where the choice is a dither threshold and not a measurement.
fn chamfer(seed: &[bool], width: usize, height: usize, outside_seeds: bool) -> Vec<u8> {
	const NEAR: u16 = 3; // an edge step, in thirds of a pixel
	const DIAG: u16 = 4; // ...and a diagonal one
	let (w, h) = (width, height);
	// In thirds of a pixel. A neighbour off the edge of the box either stands at
	// 0 (so the step to it is the whole candidate) or is not there at all.
	let mut d: Vec<u16> = seed.iter().map(|&s| if s { 0 } else { u16::MAX }).collect();
	let step = |dist: u16, s: u16| dist.saturating_add(s);
	let off = |s: u16| if outside_seeds { s } else { u16::MAX };
	for y in 0..h {
		for x in 0..w {
			let i = y * w + x;
			if d[i] == 0 {
				continue;
			}
			let (up, left, right) = (y == 0, x == 0, x + 1 == w);
			let mut m = d[i];
			m = m.min(if up || left { off(DIAG) } else { step(d[i - w - 1], DIAG) });
			m = m.min(if up { off(NEAR) } else { step(d[i - w], NEAR) });
			m = m.min(if up || right { off(DIAG) } else { step(d[i - w + 1], DIAG) });
			m = m.min(if left { off(NEAR) } else { step(d[i - 1], NEAR) });
			d[i] = m;
		}
	}
	for y in (0..h).rev() {
		for x in (0..w).rev() {
			let i = y * w + x;
			if d[i] == 0 {
				continue;
			}
			let (down, left, right) = (y + 1 == h, x == 0, x + 1 == w);
			let mut m = d[i];
			m = m.min(if down || right { off(DIAG) } else { step(d[i + w + 1], DIAG) });
			m = m.min(if down { off(NEAR) } else { step(d[i + w], NEAR) });
			m = m.min(if down || left { off(DIAG) } else { step(d[i + w - 1], DIAG) });
			m = m.min(if right { off(NEAR) } else { step(d[i + 1], NEAR) });
			d[i] = m;
		}
	}
	d.into_iter().map(|thirds| ((thirds as u32 + 1) / 3).min(255) as u8).collect()
}

/// The peak an object of this size stands at when nothing authored one: half its
/// shorter side, floored at 1 and capped at the byte the field is stored in.
///
/// A guess, and deliberately a crude one - the art carries no elevation, so the
/// only thing left to read a landform's size off is how much of the map it
/// covers. Monotone in the sprite's size, which is what the comparison
/// [`SceneryBlend::Higher`] makes actually needs; a piece whose art disagrees
/// says so through [`SceneryPiece::peak`].
pub fn default_peak(width: usize, height: usize) -> u8 {
	(width.min(height) / 2).clamp(1, 255) as u8
}

/// What share of [`default_peak`] a **low** family actually stands at, as a
/// percentage - see [`family_stands_low`].
///
/// A third. Chosen against the shipped art rather than picked round: across the
/// five packs the 76 dune, rough, rouge and meadow cut-outs guess 20..93 and the
/// 170 mountain and tree ones guess 9..183, which is not two ranges but one - the
/// tallest dune out-topping the median mountain. At a third the low families run
/// 7..31 around a median of 11, under the tall families' tenth percentile (23)
/// and well under their median (47). The overlap that remains is a big drift
/// against a small boulder, which is the right way round and is real.
const LOW_PEAK_SHARE: u32 = 33;

/// Whether a family's pieces are **low ground cover** rather than landmarks - a
/// drift, a patch of broken ground, a meadow - and so stand at
/// [`LOW_PEAK_SHARE`] of the height their footprint would otherwise imply.
///
/// By name, exactly as [`family_is_sunken`] is, and for the same reason: the name
/// is the only thing a cut-out carries that says what it *is*.
///
/// [`default_peak`] reads a landform's height off how much map it covers, which
/// is the only signal 8-bit paint leaves and is right *within* a family - a wider
/// mountain is a taller mountain. Across families it is not: a dune is a drift of
/// a metre or two however far it spreads, and a rough patch is ground rather than
/// a thing standing on it. Left unscaled they tower over the trees and mountains
/// they are meant to lie between, and `SceneryBlend::Higher` resolves every
/// overlap the wrong way round.
///
/// `rouge` is SNOW's spelling of `rough` - the ids ship that way, `piece_family`
/// reads the name it is given, and a family this list misses is a family that
/// keeps standing too tall. Renaming the assets is the tidier fix and is not this
/// one.
pub fn family_stands_low(family: &str) -> bool {
	matches!(family, "dune" | "rough" | "rouge" | "meadow")
}

/// The peak a piece of `family` stands at when nothing authored one: what its
/// footprint implies ([`default_peak`]), brought down to [`LOW_PEAK_SHARE`] for
/// a family that lies on the ground rather than standing on it.
///
/// Never `0`: a body pixel is never bare ground, so the floor is the same `1` the
/// relief itself clamps to.
pub fn family_peak(family: &str, footprint: u8) -> u8 {
	match family_stands_low(family) {
		true => ((footprint as u32 * LOW_PEAK_SHARE / 100).max(1)) as u8,
		false => footprint,
	}
}

/// Each masked pixel's brightness, box-blurred over [`LUMA_BLUR`] and then
/// stretched across `0..=255` over the range this object's own art covers.
///
/// Blurred because the source is dithered - two neighbouring pixels of one flat
/// face differ by the whole ramp - and stretched because a piece cut from a dark
/// pack would otherwise read as uniformly low ground. Only masked pixels are
/// averaged, so the silhouette's rim is not dragged down by the nothing outside
/// it.
fn stretched_luma(mask: &[bool], luma: &[u8], w: usize, h: usize) -> Vec<u8> {
	let blurred = box_mean(&luma.iter().map(|&l| l as u32).collect::<Vec<u32>>(), mask, LUMA_BLUR, w, h);
	let mean = |i: usize| blurred[i] as u8;
	let (mut lo, mut hi) = (255u8, 0u8);
	for i in 0..w * h {
		if mask[i] {
			lo = lo.min(mean(i));
			hi = hi.max(mean(i));
		}
	}
	let span = hi.saturating_sub(lo) as u32;
	(0..w * h)
		.map(|i| match () {
			// One flat tone across the whole object says nothing about its shape;
			// leave the dome to speak alone rather than invent relief.
			_ if !mask[i] || span == 0 => 128,
			_ => ((mean(i) - lo) as u32 * 255 / span) as u8,
		})
		.collect()
}

/// Each pixel's mean over the `radius`-wide box around it, counting **only
/// masked pixels** so nothing outside the region drags its edge towards zero.
/// `0` where the box holds nothing masked at all.
///
/// Separable: a horizontal run, then a vertical one over its output, both
/// carrying sums and counts so the second pass divides once at the end.
fn box_mean(values: &[u32], mask: &[bool], radius: usize, w: usize, h: usize) -> Vec<u32> {
	let (mut sum, mut count) = (vec![0u32; w * h], vec![0u32; w * h]);
	for i in 0..w * h {
		if mask[i] {
			sum[i] = values[i];
			count[i] = 1;
		}
	}
	let mut pass = |stride: usize, span: usize, other: usize| {
		let (mut s, mut c) = (vec![0u32; w * h], vec![0u32; w * h]);
		for a in 0..other {
			for b in 0..span {
				let lo = b.saturating_sub(radius);
				let hi = (b + radius).min(span - 1);
				let base = if stride == 1 { a * w } else { a };
				let (mut ts, mut tc) = (0u32, 0u32);
				for k in lo..=hi {
					let i = base + k * stride;
					ts += sum[i];
					tc += count[i];
				}
				let i = base + b * stride;
				s[i] = ts;
				c[i] = tc;
			}
		}
		(sum, count) = (s, c);
	};
	pass(1, w, h);
	pass(w, h, w);
	(0..w * h).map(|i| sum[i].checked_div(count[i]).unwrap_or(0)).collect()
}

/// The dome's share of the inferred relief, against [`LUMA_WEIGHT`]. The shape
/// leads: it is the signal that holds on every pack, and the luma refines it.
const DOME_WEIGHT: u32 = 6;
/// The blurred luma's share - see [`Sprite::height_field`].
const LUMA_WEIGHT: u32 = 4;

/// Box-blur radius, in pixels, taken over the body's ink before its brightness
/// is read as relief. Three is a little over the dither's period and well under
/// the smallest piece's short side.
const LUMA_BLUR: usize = 3;

/// How far in from the silhouette an object climbs out of the ground, in pixels
/// of [`edge_distance`]: the relief is ramped from nothing at the outline to
/// whatever the two signals say at `GROUND_SKIRT` pixels in.
///
/// **The very edge of a cut-out is where it stops being an object**, so that is
/// ground level by construction and not something to be inferred. The dome
/// already says so - it is the distance transform, near zero at the rim - but
/// the luma does not: a mountain's sunlit western flank is painted its brightest
/// ink right up to the outline, and a piece's outermost pixel was reading as
/// high ground purely because the artist lit it. Under [`SceneryBlend::Higher`]
/// that is the visible bug the relief was introduced to fix, one object over:
/// lay a hill against a mountain's flank and the hill's own bright rim wins the
/// comparison, so the silhouette stamps a hard bright line across the rock
/// instead of the two interlocking along a contour.
///
/// Three pixels, matching [`LUMA_BLUR`]: the fringe the blur smears the outside
/// world into is exactly the fringe whose luma is not to be trusted. It is a
/// hairline on a mountain and a real skirt on a boulder, which is the right way
/// round - a small object is mostly edge.
const GROUND_SKIRT: u32 = 3;

/// What [`Sprite::height_field`] needs that the sprite itself cannot say.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeightOpts<'a> {
	/// How high the object's tallest pixel stands, in map pixels.
	pub peak: u8,
	/// The body is a **depression**, not a rise - the dome becomes a
	/// [`rim_profile`], so the outline is ground level, the rim stands, and the
	/// middle sinks away behind it.
	pub sunken: bool,
	/// The body is a **pyramid**, whose shape is known before you look at it:
	/// [`pyramid_dome`] is built over the footprint and neither inferred signal
	/// is consulted. Ignored when [`sunken`](Self::sunken) - nothing is both.
	pub pyramid: bool,
	/// The body is the **face of a step in the land**, not a rise on it: ground
	/// level at the foot of the wall and the peak at its brow, which side being
	/// which read off the light ([`scarp_face`]). Ignored when
	/// [`sunken`](Self::sunken) or [`pyramid`](Self::pyramid) - a piece has one
	/// shape.
	pub scarp: bool,
	/// **Where the rim crest actually runs**, per sprite pixel, when somebody
	/// traced it on the art ([`rim_dome`]); empty when nobody did, and read only
	/// when [`sunken`](Self::sunken).
	///
	/// A **bake input**, like the templates themselves: it is the offline step
	/// that turns a traced curve into the `.hgt` a piece ships, and nothing at
	/// runtime has one. Re-tracing a rim therefore means re-running
	/// `bake_scenery --heights`, exactly as re-cutting the art means re-baking.
	pub rim: &'a [bool],
	/// **Where the foot of a wall runs**, per sprite pixel, when somebody traced
	/// it on the art - the green line to [`rim`](Self::rim)'s red, drawn along the
	/// outer edge of a cliff's band ([`scarp_rim`]).
	///
	/// Read only when [`scarp`](Self::scarp), and a bake input on the same terms
	/// as `rim`. With both lines drawn a wall's relief is **authored end to end**:
	/// ground on the green, peak on the red, and the slope between them the one
	/// the two lines imply. Empty where nobody drew it, and then the light says
	/// which flank is the low one instead.
	pub foot: &'a [bool],
}

/// Whether a family's pieces are holes in the ground rather than things on it.
///
/// By name, because that is the only thing a cut-out carries that says what it
/// *is*: the CRATER pack's `crater-*` pieces are impact craters, a raised rim
/// around a bowl, and reading their middle as a summit would stand every one of
/// them on its head. A piece whose art disagrees overrides it
/// ([`SceneryPiece::sunken`]).
pub fn family_is_sunken(family: &str) -> bool {
	matches!(family, "crater" | "pit" | "hole")
}

// ----- blending a piece into its own kind -------------------------------------

/// How wide the dithered transition is, in map pixels, where a piece is laid
/// over one of its **own family**.
///
/// A quarter of a tile: wide enough to read as a gradient at 1:1, narrow enough
/// that the smallest hill (33x28) keeps a solid core.
pub const BLEND_BAND: u8 = 16;

/// The 8x8 ordered-dither (Bayer) matrix, in `0..64`.
#[rustfmt::skip]
const BAYER8: [u8; 64] = [
	 0, 32,  8, 40,  2, 34, 10, 42,
	48, 16, 56, 24, 50, 18, 58, 26,
	12, 44,  4, 36, 14, 46,  6, 38,
	60, 28, 52, 20, 62, 30, 54, 22,
	 3, 35, 11, 43,  1, 33,  9, 41,
	51, 19, 59, 27, 49, 17, 57, 25,
	15, 47,  7, 39, 13, 45,  5, 37,
	63, 31, 55, 23, 61, 29, 53, 21,
];

/// A piece's **family**: the first word of its display name, lowercased -
/// `"Mountain 12"` and `"Mountain 3"` are both `"mountain"`.
///
/// Two pieces of one family are two faces of the same landform, so overlapping
/// them is meant to build a bigger one and the seam between them is dithered
/// away ([`blend_keeps`]). Two pieces of *different* families are different
/// things, and one hides the other exactly as it did before.
pub fn piece_family(name: &str) -> String {
	name.split_whitespace().next().unwrap_or_default().to_lowercase()
}

/// Whether a piece keeps its ink at a pixel that one of its own family already
/// covers - the dither that turns the seam into a gradient.
///
/// `edge` is how deep inside its own silhouette the pixel sits
/// ([`Sprite::edge_distance`]) and `(x, y)` is the pixel in **sprite-local**
/// coordinates, so the pattern rides with the object rather than crawling under
/// it when the map scrolls. The rim (`edge == 0`) always gives way and anything
/// [`BLEND_BAND`] deep always wins; in between the ordered dither passes a
/// fraction `edge / BLEND_BAND` of the pixels.
///
/// `shaders/scenery.wgsl` reproduces this exactly, in the same integer
/// arithmetic - the screen and the WRL export may not disagree about where an
/// object's ink lands.
pub fn blend_keeps(edge: u8, x: u32, y: u32) -> bool {
	if edge >= BLEND_BAND {
		return true;
	}
	let bayer = BAYER8[(y % 8) as usize * 8 + (x % 8) as usize] as u32;
	edge as u32 * 64 > bayer * BLEND_BAND as u32
}

const CLASS_GROUND: u8 = 0;
const CLASS_SHADE: u8 = 1;
const CLASS_BODY: u8 = 2;

/// Cut an object out of `src`, a `width * height` box of palette indices where
/// `None` is a hole the source never painted (an empty template cell).
pub fn cut(
	src: &[Option<u8>],
	width: usize,
	height: usize,
	ground: &GroundInk,
	shadow: &ShadowInk,
	opts: &CutOpts,
) -> Sprite {
	assert_eq!(src.len(), width * height, "cut: src is not width * height");
	if width == 0 || height == 0 {
		return Sprite::default();
	}

	let class: Vec<u8> = src
		.iter()
		.map(|p| match *p {
			None => CLASS_GROUND,
			Some(i) if ground.contains(i) => CLASS_GROUND,
			Some(i) if shadow.contains(i) => CLASS_SHADE,
			Some(_) => CLASS_BODY,
		})
		.collect();

	// Seal = the body mask closed by `opts.close`. Only the flood consults it;
	// the pixels it adds are not themselves body.
	let mut seal: Vec<u8> = class.iter().map(|&c| u8::from(c == CLASS_BODY)).collect();
	for _ in 0..opts.close {
		seal = morph(&seal, width, height, true);
	}
	for _ in 0..opts.close {
		seal = morph(&seal, width, height, false);
	}

	// Ground that is not reachable from the edge is ground the object encloses -
	// its own lit face, or snow caught inside a rock - and stays with it.
	let outside = flood_from_edge(&class, &seal, width, height);

	// The flood decides the fate of *ground* only. Shadow is shadow wherever it
	// falls: a cast shadow reaches away from the object, so a seal wide enough
	// to close the object's dithered edge inevitably swallows the near end of it
	// - and an enclosed shadow pixel turned opaque would be a hard black patch
	// in the middle of an otherwise translucent shadow.
	let mut body = vec![0u8; width * height];
	let mut shade = vec![0u8; width * height];
	for i in 0..width * height {
		let Some(index) = src[i] else { continue };
		match class[i] {
			CLASS_BODY => body[i] = index,
			CLASS_SHADE => shade[i] = opts.alpha,
			_ if !outside[i] => body[i] = index,
			_ => {}
		}
	}

	crop(body, shade, width, height)
}

/// One 3x3 dilate (`grow`) or erode pass, clamping at the edge so an object
/// that touches the border is not eroded away from outside the box.
fn morph(mask: &[u8], width: usize, height: usize, grow: bool) -> Vec<u8> {
	let mut out = vec![0u8; mask.len()];
	for y in 0..height {
		for x in 0..width {
			let mut acc = u8::from(!grow);
			for dy in -1i32..=1 {
				for dx in -1i32..=1 {
					let nx = (x as i32 + dx).clamp(0, width as i32 - 1) as usize;
					let ny = (y as i32 + dy).clamp(0, height as i32 - 1) as usize;
					let v = mask[ny * width + nx];
					acc = if grow { acc | v } else { acc & v };
				}
			}
			out[y * width + x] = acc;
		}
	}
	out
}

/// 4-connected flood from every edge pixel through everything that is neither
/// body nor sealed. What it reaches is the ground the object sits on.
fn flood_from_edge(class: &[u8], seal: &[u8], width: usize, height: usize) -> Vec<bool> {
	let mut outside = vec![false; class.len()];
	let mut stack: Vec<usize> = Vec::new();
	let push = |i: usize, outside: &mut Vec<bool>, stack: &mut Vec<usize>| {
		if !outside[i] && class[i] != CLASS_BODY && seal[i] == 0 {
			outside[i] = true;
			stack.push(i);
		}
	};
	for x in 0..width {
		push(x, &mut outside, &mut stack);
		push((height - 1) * width + x, &mut outside, &mut stack);
	}
	for y in 0..height {
		push(y * width, &mut outside, &mut stack);
		push(y * width + width - 1, &mut outside, &mut stack);
	}
	while let Some(i) = stack.pop() {
		let (x, y) = (i % width, i / width);
		if x > 0 {
			push(i - 1, &mut outside, &mut stack);
		}
		if x + 1 < width {
			push(i + 1, &mut outside, &mut stack);
		}
		if y > 0 {
			push(i - width, &mut outside, &mut stack);
		}
		if y + 1 < height {
			push(i + width, &mut outside, &mut stack);
		}
	}
	outside
}

/// Trim the empty margin off both planes, recording where the remainder sat.
fn crop(body: Vec<u8>, shade: Vec<u8>, width: usize, height: usize) -> Sprite {
	let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0usize, 0usize);
	for y in 0..height {
		for x in 0..width {
			let i = y * width + x;
			if body[i] == 0 && shade[i] == 0 {
				continue;
			}
			x0 = x0.min(x);
			y0 = y0.min(y);
			x1 = x1.max(x);
			y1 = y1.max(y);
		}
	}
	if x0 > x1 || y0 > y1 {
		return Sprite::default();
	}
	let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
	let mut out = Sprite {
		width: w as u16,
		height: h as u16,
		origin_x: x0 as u16,
		origin_y: y0 as u16,
		body: Vec::with_capacity(w * h),
		shade: Vec::with_capacity(w * h),
	};
	for y in y0..=y1 {
		let row = y * width;
		out.body.extend_from_slice(&body[row + x0..row + x1 + 1]);
		out.shade.extend_from_slice(&shade[row + x0..row + x1 + 1]);
	}
	out
}

// ----- plane encoding ---------------------------------------------------------

/// Row-run encoding for a sprite plane: per row, `(skip, len, bytes…)` triples
/// until the row is spent - `skip` zero bytes, then `len` literal bytes, both
/// `u16` little-endian. Trailing zeros cost one empty triple.
///
/// A cut sprite is mostly holes (the ground that came out, the template's empty
/// cells), so the shipped `.bin` is a fraction of `width * height` per plane.
pub fn encode_plane(plane: &[u8], width: usize, height: usize) -> Vec<u8> {
	assert_eq!(plane.len(), width * height, "encode_plane: not width * height");
	let mut out = Vec::new();
	for y in 0..height {
		let row = &plane[y * width..(y + 1) * width];
		let mut x = 0usize;
		while x < width {
			let skip = row[x..].iter().take_while(|&&b| b == 0).count();
			x += skip;
			let len = row[x..].iter().take_while(|&&b| b != 0).count();
			out.extend_from_slice(&(skip as u16).to_le_bytes());
			out.extend_from_slice(&(len as u16).to_le_bytes());
			out.extend_from_slice(&row[x..x + len]);
			x += len;
		}
	}
	out
}

/// Inverse of [`encode_plane`]. Errors rather than panics - it parses shipped
/// bytes.
pub fn decode_plane(data: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
	let mut plane = vec![0u8; width * height];
	let mut at = 0usize;
	for y in 0..height {
		let mut x = 0usize;
		while x < width {
			if at + 4 > data.len() {
				return Err(format!("scenery plane: truncated header at row {y}"));
			}
			let skip = u16::from_le_bytes([data[at], data[at + 1]]) as usize;
			let len = u16::from_le_bytes([data[at + 2], data[at + 3]]) as usize;
			at += 4;
			if x + skip + len > width {
				return Err(format!("scenery plane: run overruns row {y}"));
			}
			if at + len > data.len() {
				return Err(format!("scenery plane: truncated run at row {y}"));
			}
			x += skip;
			plane[y * width + x..y * width + x + len].copy_from_slice(&data[at..at + len]);
			at += len;
			x += len;
			if skip == 0 && len == 0 {
				return Err(format!("scenery plane: empty run at row {y} would not terminate"));
			}
		}
	}
	Ok(plane)
}

// ----- shading an indexed pixel -----------------------------------------------

/// Where every palette index lands once a scenery shadow of one alpha falls on
/// it.
///
/// On screen a shadow is a translucent quad, but a WRL tile holds palette
/// indices, not colours - so the export has to re-quantize each shadowed pixel.
/// Answering that once per alpha keeps the bake off a 256-entry nearest-colour
/// search per shadow pixel, and keeps the exported map agreeing with what the
/// editor drew.
///
/// The darkening runs in **linear** space, because that is where the editor's
/// alpha blend against an sRGB render target runs. Multiplying the 8-bit values
/// instead - the space the art was authored in - is a defensible reading of the
/// original palette, but it made the preview and the export differ by ~20 units
/// on every shadow pixel. With the alpha re-derived per space the two land on
/// the same palette ink anyway, so agreement is free.
#[derive(Clone)]
pub struct ShadeTable {
	map: [u8; 256],
}

impl ShadeTable {
	pub fn build(palette: &[u8], alpha: u8) -> Self {
		let keep = 1.0 - alpha as f64 / 255.0;
		let mut map = [0u8; 256];
		for index in 0..=255u8 {
			let lit = slot_rgb(palette, index);
			let want = [0, 1, 2].map(|c| to_srgb(to_linear(lit[c]) * keep));
			map[index as usize] = nearest_shade_slot(palette, want);
		}
		Self { map }
	}

	pub fn apply(&self, index: u8) -> u8 {
		self.map[index as usize]
	}
}

/// One 8-bit sRGB channel as linear light.
fn to_linear(v: u8) -> f64 {
	let v = v as f64 / 255.0;
	if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Linear light back to an 8-bit sRGB channel.
fn to_srgb(v: f64) -> u8 {
	let v = v.clamp(0.0, 1.0);
	let e = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
	(e * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The linear-space alpha that best turns `from` into `to` - how an sRGB-space
/// observation of the art ("this ink is 65% of the ground") becomes the number
/// a blend can apply. Searched rather than solved: no single alpha reproduces
/// an 8-bit multiply on all three channels, so the best least-squares one is
/// the honest answer.
fn linear_alpha_for(from: [f64; 3], to: [f64; 3]) -> u8 {
	let mut best = 0u8;
	let mut best_err = f64::MAX;
	for alpha in 0..=255u8 {
		let keep = 1.0 - alpha as f64 / 255.0;
		let err: f64 = (0..3)
			.map(|c| {
				let got = to_srgb(to_linear(from[c].clamp(0.0, 255.0).round() as u8) * keep) as f64;
				(got - to[c]).powi(2)
			})
			.sum();
		if err < best_err {
			best_err = err;
			best = alpha;
		}
	}
	best
}

/// The nearest slot to `rgb` that a shadow may land on: minimum squared RGB
/// distance over every slot that is neither transparent (0) nor colour-cycled
/// by the engine. A shadow parked on an animated slot would shimmer in-game,
/// the same reason `deanimate` moves land art off those slots. Ties resolve to
/// the lowest slot, so the table is deterministic.
fn nearest_shade_slot(palette: &[u8], rgb: [u8; 3]) -> u8 {
	let mut best = 0u8;
	let mut best_d = u32::MAX;
	for slot in 0..=255u8 {
		if slot == 0 || crate::deanimate::animated_slot(slot) {
			continue;
		}
		let c = slot_rgb(palette, slot);
		let d: u32 = (0..3).map(|i| (c[i] as i32 - rgb[i] as i32).pow(2) as u32).sum();
		if d < best_d {
			best_d = d;
			best = slot;
		}
	}
	best
}

// ----- the shipped asset ------------------------------------------------------

/// The asset directory a baked pack lives under, relative to the resources
/// root: `resources/assets/scenery/<PACK>/`.
///
/// **A library is a directory of pieces, not one packed file** - the same shape
/// `templates/<PACK>/` has, and for the same reason: a user adds a cut-out by
/// dropping files in, and edits one by opening one of them. Each piece is up to
/// three files under its own id:
///
/// | file | holds | required |
/// |---|---|---|
/// | `<id>.scn` | the piece: meta and the body / shade planes ([`SCN_MAGIC`]) | yes |
/// | `<id>.json` | the hand-editable meta - name, family, transform, pass, relief | no |
/// | `<id>.hgt` | the authored height map ([`HGT_MAGIC`]) | no |
///
/// The `.scn` is the piece and is self-contained, which is what makes it the
/// same file the export key writes and the import key reads. The other two are
/// **overlays loaded when present**: the `.json` wins over the meta inside the
/// `.scn` (so editing a name is editing a text file), and the `.hgt` wins over
/// the relief bundled in it (so drawing a height map is dropping one in). The
/// **file stem is the id** - the name in the `.scn` is not consulted - so
/// renaming the files renames the piece, and no index has to be kept in step.
pub const SCENERY_DIR: &str = "scenery";

/// The format version [`SceneryPack::save`] writes and [`SceneryPack::load`]
/// accepts.
pub const SCENERY_VERSION: &str = "1";

/// Pass value recorded for a template cell that holds no tile.
pub const PASS_EMPTY: u8 = 255;

/// One scenery object placed on a map.
///
/// Anchored by the **footprint origin** in map pixels - where cell (0,0) of the
/// source template sits - not by the sprite's own top-left. The sprite is
/// cropped, so its top-left moves whenever the cut is re-tuned; anchoring to
/// the footprint keeps a saved placement where the user put it across a
/// re-bake. Signed, so an object may hang off the map's left or top edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenerySpot {
	/// The tile pack whose scenery library holds the piece (`SceneryPack::pack`).
	pub pack: String,
	/// The piece's id in that library ([`SceneryPiece::id`]).
	pub piece: String,
	pub x: i32,
	pub y: i32,
	/// How this placement's ink meets the scenery already under it.
	pub blend: SceneryBlend,
}

/// What a placement does where its ink lands on **another placement's** ink.
///
/// Only scenery is counted: the ground underneath is never part of the
/// comparison, and a pixel no other placement covers is always the placement's
/// own. The choice is between two whole palette inks, never a mix of them - the
/// WRL export has to write one index per pixel, so a blend that invented a
/// colour could not be exported at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SceneryBlend {
	/// Paint over: the later placement's ink wins, as it always did.
	#[default]
	Normal,
	/// Keep whichever ink is lighter - the one that reads as lit.
	Brighter,
	/// Keep whichever ink is darker - the one that reads as shadowed.
	Darker,
	/// Keep the ink of whichever object stands **higher** at that pixel
	/// ([`Sprite::height_field`]), so two landforms interlock instead of one
	/// flatly covering the other: a hill laid against a mountain's flank shows
	/// only where it actually rises above it.
	///
	/// The heights are inferred from the art and are honest guesses, not
	/// measurements - a piece whose own says otherwise carries
	/// [`SceneryPiece::peak`]. Equal heights keep the newer placement's ink, so
	/// the answer is a total order and two coincident copies of one piece render
	/// as one.
	Higher,
}

impl SceneryBlend {
	/// The lowercase name the console and the project file use.
	pub fn name(self) -> &'static str {
		match self {
			Self::Normal => "normal",
			Self::Brighter => "brighter",
			Self::Darker => "darker",
			Self::Higher => "higher",
		}
	}

	pub fn parse(text: &str) -> Option<Self> {
		match text {
			"normal" => Some(Self::Normal),
			"brighter" => Some(Self::Brighter),
			"darker" => Some(Self::Darker),
			"higher" => Some(Self::Higher),
			_ => None,
		}
	}

	/// The four, in menu order.
	pub const ALL: [Self; 4] = [Self::Normal, Self::Brighter, Self::Darker, Self::Higher];

	/// Which of two inks this mode keeps. `over` is the ink being painted,
	/// `under` what is already there, and `over_h` / `under_h` how high the two
	/// objects stand at that pixel ([`Sprite::height_field`]).
	///
	/// Two inks of equal brightness are ordered by index, and two equal heights
	/// keep `over`, so the answer is a total order and not a coin toss - which
	/// is what lets the renderer reduce the same decision to `max` / `min`
	/// blending on [`ink_ranks`] and to a depth test on the heights.
	pub fn pick(self, over: u8, under: u8, over_h: u8, under_h: u8, brightness: &[u8; 256]) -> u8 {
		let key = |i: u8| ((brightness[i as usize] as u16) << 8) | i as u16;
		match self {
			Self::Normal => over,
			Self::Brighter if key(under) > key(over) => under,
			Self::Darker if key(under) < key(over) => under,
			Self::Higher if under_h > over_h => under,
			_ => over,
		}
	}
}

/// The palette's inks in brightness order, as `(rank of each ink, ink of each
/// rank)` - the form [`SceneryBlend`] takes on the GPU, where "keep the brighter
/// of two inks" has to be a blend op rather than a comparison.
///
/// Ranked over indices `1..=255` only: `0` is "no ink here" in a sprite's body
/// plane, so a layer can hold `rank + 1` and keep `0` for "no scenery". Ties
/// break by index, exactly as [`SceneryBlend::pick`] breaks them.
pub fn ink_ranks(palette: &[u8]) -> ([u8; 256], [u8; 256]) {
	let brightness = brightness_table(palette);
	let mut inks: Vec<u8> = (1..=255).collect();
	inks.sort_by_key(|&i| ((brightness[i as usize] as u16) << 8) | i as u16);
	let (mut rank_of, mut ink_of) = ([0u8; 256], [0u8; 256]);
	for (rank, &ink) in inks.iter().enumerate() {
		rank_of[ink as usize] = rank as u8;
		ink_of[rank] = ink;
	}
	(rank_of, ink_of)
}

/// Per-index brightness for [`SceneryBlend::pick`], on a palette of 256 RGB
/// triples: Rec. 601 luma, the same weighting the minimap and the palette panel
/// sort by, quantized to a byte so the editor and the export compare identical
/// numbers.
pub fn brightness_table(palette: &[u8]) -> [u8; 256] {
	std::array::from_fn(|i| {
		let rgb = slot_rgb(palette, i as u8);
		let luma = 0.299 * rgb[0] as f32 + 0.587 * rgb[1] as f32 + 0.114 * rgb[2] as f32;
		luma.round().clamp(0.0, 255.0) as u8
	})
}

/// One placeable object.
#[derive(Clone, Debug)]
pub struct SceneryPiece {
	/// The source template's file stem (`"mountain-3"`) - stable across
	/// renames of the display name, and what a placement stores.
	pub id: String,
	/// Display name, from the template's `name`.
	pub name: String,
	/// What kind of thing this is - `"mountain"`, `"hill"`, `"trees"`. Written
	/// into the manifest, and prefilled from the display name's first word
	/// ([`piece_family`]) when a library predates the field, so the two never
	/// disagree by accident. Pieces of one family are meant to be laid over each
	/// other; pieces of different families are different things.
	pub family: String,
	/// How a placement of this piece may be rotated or mirrored - the same four
	/// values a tile family carries (`tiles.props.json` `transformable`).
	/// Authored per piece, defaulting to [`Transformable::No`]: nothing acts on
	/// it yet, so it is data the manifest carries, not behaviour.
	pub transformable: Transformable,
	/// How high this object's tallest pixel stands, in map pixels - what
	/// [`SceneryBlend::Higher`] compares two overlapping placements by.
	/// `None` = inferred from the sprite ([`Sprite::default_peak`]), which is
	/// what every shipped cut-out uses; authored only where the guess is wrong.
	pub peak: Option<u8>,
	/// This object is a **depression** rather than a rise. `None` = inferred
	/// from the family ([`family_is_sunken`]).
	pub sunken: Option<bool>,
	/// This object is the **face of a step in the land** - a cliff, whose foot
	/// is ground level and whose brow is the peak ([`scarp_face`]). `None` =
	/// inferred from the family ([`family_is_scarp`]), which no shipped family
	/// is: a wall is marked per piece.
	pub scarp: Option<bool>,
	/// The **authored relief**: one byte of elevation per sprite pixel, in the
	/// sprite's own `width * height` frame, or `None` to infer the whole field
	/// from the art ([`Sprite::height_field`]).
	///
	/// This is the difference between a guess and a measurement. Everything
	/// [`Sprite::height_field`] does is an inference from what the art happens to
	/// record - a dome off the silhouette, a lit face off the palette - because
	/// nothing in 8-bit indexed paint ever said how tall a mountain is. A piece
	/// that carries this field was drawn a real height map instead, and the
	/// inference is not consulted at all.
	///
	/// Read from `<id>.hgt` beside the piece and absent when that file is
	/// (`SceneryPack::load`), so a library of pieces that all infer their relief
	/// is exactly a library from before any of this existed. A plane of the wrong
	/// length is refused at load and ignored at use - see
	/// [`Self::height_field`](Self::height_field).
	pub height: Option<Vec<u8>>,
	/// The source template's footprint in cells. The sprite is cropped, so this
	/// is bigger than `sprite.width / 64` in general.
	pub cells_w: u16,
	pub cells_h: u16,
	/// Row-major `cells_w * cells_h` pass values from the source tiles, so a
	/// placement can decide what it blocks without re-reading the template.
	/// [`PASS_EMPTY`] where the template had a hole.
	pub pass: Vec<u8>,
	pub sprite: Sprite,
	/// This piece was authored by the user (it loaded from the **user** scenery
	/// root, not the shipped one) - so it may be renamed and deleted. Derived
	/// from where it was read, never stored in the manifest: the same library
	/// file is shipped read-only and kept read-write, and which one you have is
	/// a property of the install, not of the piece.
	pub user: bool,
}

impl SceneryPiece {
	/// Where the sprite's top-left lands when this piece is anchored at `spot`
	/// (the crop offset added to the footprint origin).
	pub fn sprite_origin(&self, spot: &ScenerySpot) -> (i32, i32) {
		(spot.x + self.sprite.origin_x as i32, spot.y + self.sprite.origin_y as i32)
	}

	/// The footprint origin that puts this piece's **centre of mass** on map
	/// pixel `(px, py)` - what a click means when the piece rides the cursor.
	///
	/// The inverse of [`Self::sprite_origin`] through
	/// [`Sprite::center_of_mass`]: place at this and the piece looks centred on
	/// the point you clicked, whatever its silhouette. Free placement, so this
	/// is a pixel and nothing rounds to a cell.
	pub fn centered_at(&self, px: i32, py: i32) -> (i32, i32) {
		let (cx, cy) = self.sprite.center_of_mass();
		(px - self.sprite.origin_x as i32 - cx, py - self.sprite.origin_y as i32 - cy)
	}

	/// How this piece's relief is measured - the authored values where it has
	/// them, the inferred ones where it does not.
	///
	/// No [`rim`](HeightOpts::rim) and no [`foot`](HeightOpts::foot): a traced
	/// curve is a bake input and a piece does not carry one. What the tracing
	/// produced is the piece's `.hgt`, and that is read straight off disk by
	/// [`Self::height_field`] before any of this is consulted.
	pub fn height_opts(&self) -> HeightOpts<'static> {
		HeightOpts {
			peak: self.peak.unwrap_or_else(|| family_peak(&self.family, self.sprite.default_peak())),
			sunken: self.sunken.unwrap_or_else(|| family_is_sunken(&self.family)),
			pyramid: family_is_pyramid(&self.family),
			scarp: self.scarp.unwrap_or_else(|| family_is_scarp(&self.family)),
			rim: &[],
			foot: &[],
		}
	}

	/// **How high this piece stands, per sprite pixel** - the authored
	/// [`Self::height`] where the piece has one, and [`Sprite::height_field`]'s
	/// inference where it does not.
	///
	/// The fallback is the normal case, not an error path: only a piece somebody
	/// drew a height map for carries one, and a library that carries none
	/// renders exactly as it did before relief could be authored at all. A plane
	/// whose length does not match the sprite is treated as absent rather than
	/// panicking - it can only come from a `.hgt` edited out from under a piece
	/// whose art was re-cut, and a wrong-sized relief is worth less than the
	/// guess it would replace.
	pub fn height_field(&self, brightness: &[u8; 256]) -> Vec<u8> {
		let texels = self.sprite.width as usize * self.sprite.height as usize;
		match &self.height {
			Some(height) if height.len() == texels => height.clone(),
			_ => self.sprite.height_field(brightness, &self.height_opts()),
		}
	}

	/// Whether this piece's relief was **authored** (a `.hgt` beside it) rather
	/// than inferred from its art - what the dialog's Heightmap tab reports and
	/// the bake's summary counts.
	pub fn height_authored(&self) -> bool {
		self.height.as_ref().is_some_and(|h| h.len() == self.sprite.width as usize * self.sprite.height as usize)
	}

	/// This piece's ink at a **sprite-local** pixel as `(body, shade)`; `(0, 0)`
	/// outside the sprite. The two are mutually exclusive by construction.
	pub fn texel(&self, lx: i32, ly: i32) -> (u8, u8) {
		if lx < 0 || ly < 0 || lx >= self.sprite.width as i32 || ly >= self.sprite.height as i32 {
			return (0, 0);
		}
		let i = ly as usize * self.sprite.width as usize + lx as usize;
		(self.sprite.body[i], self.sprite.shade[i])
	}

	/// The pass value this piece imposes on the map cell whose centre is at
	/// `(px, py)` map pixels, given the piece is anchored at `spot`.
	/// `None` when the centre falls outside the footprint or on a cell the
	/// source template left empty.
	pub fn pass_under(&self, spot: &ScenerySpot, px: i32, py: i32) -> Option<u8> {
		let (cx, cy) = ((px - spot.x).div_euclid(64), (py - spot.y).div_euclid(64));
		if cx < 0 || cy < 0 || cx >= self.cells_w as i32 || cy >= self.cells_h as i32 {
			return None;
		}
		match self.pass[cy as usize * self.cells_w as usize + cx as usize] {
			PASS_EMPTY => None,
			value => Some(value),
		}
	}
}

/// Every object cut from one tile pack's templates.
#[derive(Clone, Debug)]
pub struct SceneryPack {
	/// The tile pack the art (and so the palette) comes from.
	pub pack: String,
	pub pieces: Vec<SceneryPiece>,
}

impl SceneryPack {
	pub fn piece(&self, id: &str) -> Option<&SceneryPiece> {
		self.pieces.iter().find(|p| p.id == id)
	}

	/// Read `<root>/scenery/<pack>/` - every `<id>.scn` in it, in id order, with
	/// the `<id>.json` and `<id>.hgt` beside each one laid over it.
	///
	/// Errors only on a directory that will not list or a `.scn` that will not
	/// parse: a piece is a file, so one bad file is a bad library and saying
	/// which is more use than dropping it. The two overlays are optional by
	/// design and a `.hgt` drawn for a different frame is **dropped** rather than
	/// fatal - it can only be a height map left beside re-cut art, and falling
	/// back to the inference is what a piece without one already does.
	pub fn load(root: &Path, pack: &str) -> Result<Self, String> {
		let dir = root.join(SCENERY_DIR).join(pack);
		let mut ids: Vec<String> = std::fs::read_dir(&dir)
			.map_err(|e| format!("read {}: {e}", dir.display()))?
			.filter_map(|e| e.ok().map(|e| e.path()))
			.filter(|p| p.extension().is_some_and(|x| x == SCN_EXT))
			.filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
			.collect();
		ids.sort();
		let mut pieces = Vec::with_capacity(ids.len());
		for id in ids {
			pieces.push(load_piece(&dir, &id)?);
		}
		Ok(Self { pack: pack.to_string(), pieces })
	}

	/// One pack's library as the editor sees it: the shipped cut-outs, then the
	/// user's own from `user_root` with [`SceneryPiece::user`] set. `None` when
	/// neither root holds one, so a pack that ships no scenery and has none
	/// authored simply does not appear.
	///
	/// A user id that collides with a shipped one **replaces** it, the way a
	/// user tile pack shadows the tiles it re-uses: the alternative is two
	/// pieces answering to one name, and a placement names its piece by string.
	pub fn load_merged(assets_root: &Path, user_root: &Path, pack: &str) -> Option<Self> {
		let mut lib = Self::load(assets_root, pack).unwrap_or_else(|_| Self { pack: pack.to_string(), pieces: vec![] });
		let user = Self::load(user_root, pack).map(|l| l.pieces).unwrap_or_default();
		for piece in user {
			let piece = SceneryPiece { user: true, ..piece };
			match lib.pieces.iter().position(|p| p.id == piece.id) {
				Some(i) => lib.pieces[i] = piece,
				None => lib.pieces.push(piece),
			}
		}
		(!lib.pieces.is_empty()).then_some(lib)
	}

	/// Just this library's user-authored pieces, as a pack that can be written
	/// back to the user root. The shipped set is never rewritten, so a save is
	/// always the whole of what the user owns.
	pub fn user_subset(&self) -> Self {
		Self { pack: self.pack.clone(), pieces: self.pieces.iter().filter(|p| p.user).cloned().collect() }
	}

	/// Write `<root>/scenery/<pack>/`, creating it if needed: three files per
	/// piece, and **every file of a piece this library no longer holds removed**.
	///
	/// The prune is what makes a delete a delete. A library is written as a
	/// whole - the caller loads it, changes one piece, and saves it back - so a
	/// stem with a `.scn` that no piece claims is a piece that was dropped. Only
	/// such a stem's three files go; anything else in the directory (a pack's
	/// `tune.json`, a user's own notes) is not a piece and is left alone.
	pub fn save(&self, root: &Path) -> Result<(), String> {
		let dir = root.join(SCENERY_DIR).join(&self.pack);
		std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
		for piece in &self.pieces {
			save_piece(&dir, &self.pack, piece)?;
		}
		let stale: Vec<String> = std::fs::read_dir(&dir)
			.map_err(|e| format!("read {}: {e}", dir.display()))?
			.filter_map(|e| e.ok().map(|e| e.path()))
			.filter(|p| p.extension().is_some_and(|x| x == SCN_EXT))
			.filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
			.filter(|id| !self.pieces.iter().any(|p| &p.id == id))
			.collect();
		for id in stale {
			for ext in [SCN_EXT, "json", HGT_EXT] {
				let path = dir.join(format!("{id}.{ext}"));
				if path.is_file() {
					std::fs::remove_file(&path).map_err(|e| format!("remove {}: {e}", path.display()))?;
				}
			}
		}
		Ok(())
	}
}

/// One piece off disk: its `.scn`, with the `.json` and `.hgt` beside it laid
/// over it. The **file stem is the id**, whatever the `.scn` calls itself.
fn load_piece(dir: &Path, id: &str) -> Result<SceneryPiece, String> {
	let path = dir.join(format!("{id}.{SCN_EXT}"));
	let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
	let (piece, _) = read_scn(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
	// `read_scn` answers the import key, where a piece is the user's by
	// definition; whose a *library's* piece is depends on which root it came out
	// of, and only `load_merged` knows that.
	let mut piece = SceneryPiece { id: id.to_string(), user: false, ..piece };

	let meta = dir.join(format!("{id}.json"));
	if meta.is_file() {
		let text = std::fs::read_to_string(&meta).map_err(|e| format!("read {}: {e}", meta.display()))?;
		apply_meta(&mut piece, &json::parse(&text)?).map_err(|e| format!("{}: {e}", meta.display()))?;
	}

	let relief = dir.join(format!("{id}.{HGT_EXT}"));
	if relief.is_file() {
		let bytes = std::fs::read(&relief).map_err(|e| format!("read {}: {e}", relief.display()))?;
		let (plane, w, h) = read_hgt(&bytes).map_err(|e| format!("{}: {e}", relief.display()))?;
		// A height map drawn for a frame the art no longer has is a stale file,
		// not a corrupt one: drop it and let the piece infer, exactly as a piece
		// with no height map at all does.
		if (w, h) == (piece.sprite.width, piece.sprite.height) {
			piece.height = Some(plane);
		}
	}
	Ok(piece)
}

/// Write one piece's files. The `.hgt` is written only for an authored relief,
/// and **removed** when there is none, so turning a height map off is turning
/// the file off.
fn save_piece(dir: &Path, pack: &str, piece: &SceneryPiece) -> Result<(), String> {
	let write = |name: String, bytes: &[u8]| -> Result<(), String> {
		let path = dir.join(name);
		std::fs::write(&path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
	};
	write(format!("{}.{SCN_EXT}", piece.id), &write_scn(piece, pack))?;
	write(format!("{}.json", piece.id), piece_meta(piece).as_bytes())?;
	let relief = dir.join(format!("{}.{HGT_EXT}", piece.id));
	match &piece.height {
		Some(plane) if piece.height_authored() => {
			write(format!("{}.{HGT_EXT}", piece.id), &write_hgt(plane, piece.sprite.width, piece.sprite.height))?;
		}
		_ if relief.is_file() => {
			std::fs::remove_file(&relief).map_err(|e| format!("remove {}: {e}", relief.display()))?;
		}
		_ => {}
	}
	Ok(())
}

/// The hand-editable half of a piece, as `<id>.json`: what somebody would
/// reasonably want to change without a paint program open.
///
/// Deliberately **not** the whole piece - the geometry and the planes stay in
/// the `.scn`, where they cannot be edited into disagreeing with the art. What
/// is here is what [`apply_meta`] reads back.
fn piece_meta(piece: &SceneryPiece) -> String {
	use json::JsonValue as J;
	let mut entry = vec![
		("version".to_string(), J::String(SCENERY_VERSION.to_string())),
		("id".to_string(), J::String(piece.id.clone())),
		("name".to_string(), J::String(piece.name.clone())),
		("family".to_string(), J::String(piece.family.clone())),
		("transform".to_string(), transform_value(piece.transformable)),
		("pass".to_string(), J::Array(pass_rows(piece))),
	];
	// The relief, written only where it was authored - `auto` is the absence of
	// both keys, exactly as it is inside the `.scn`.
	if let Some(peak) = piece.peak {
		entry.push(("peak".to_string(), J::Number(peak as f64)));
	}
	if let Some(sunken) = piece.sunken {
		entry.push(("sunken".to_string(), J::Bool(sunken)));
	}
	if let Some(scarp) = piece.scarp {
		entry.push(("scarp".to_string(), J::Bool(scarp)));
	}
	J::Object(entry).to_pretty()
}

/// Lay a `<id>.json` over the piece its `.scn` decoded to. Every field is
/// optional: the file may say as little as a new name.
fn apply_meta(piece: &mut SceneryPiece, value: &json::JsonValue) -> Result<(), String> {
	if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
		piece.name = name.to_string();
	}
	if let Some(family) = value.get("family").and_then(|v| v.as_str()) {
		piece.family = family.to_string();
	}
	// `transform` is the name a piece's file uses; `transformable` is the name
	// `tiles.props.json` uses for the very same four values, and a file that
	// says it that way means the same thing.
	if let Some(v) = value.get("transform").or_else(|| value.get("transformable")) {
		piece.transformable = parse_transform(v).ok_or("bad transform (true|false|\"sync\"|\"invert\")")?;
	}
	if let Some(rows) = value.get("pass").and_then(|v| v.as_array()) {
		piece.pass = parse_pass(rows, piece.cells_w as usize, piece.cells_h as usize)?;
	}
	// Absent leaves the `.scn`'s own value alone; `null` is how a file says
	// "infer it", which is the one way back from an authored number in a text
	// editor.
	if let Some(v) = value.get("peak") {
		piece.peak = match v {
			json::JsonValue::Null => None,
			v => Some(v.as_f64().filter(|f| (1.0..=255.0).contains(f)).ok_or("bad peak (1..=255)")? as u8),
		};
	}
	if let Some(v) = value.get("sunken") {
		piece.sunken = match v {
			json::JsonValue::Null => None,
			v => Some(v.as_bool().ok_or("bad sunken (true|false)")?),
		};
	}
	if let Some(v) = value.get("scarp") {
		piece.scarp = match v {
			json::JsonValue::Null => None,
			v => Some(v.as_bool().ok_or("bad scarp (true|false)")?),
		};
	}
	Ok(())
}

fn num_pair<T: Into<f64>>(a: T, b: T) -> json::JsonValue {
	json::JsonValue::Array(vec![json::JsonValue::Number(a.into()), json::JsonValue::Number(b.into())])
}

/// How a piece may be rotated or mirrored, in the four spellings
/// `tiles.props.json` already uses for a tile family.
fn transform_value(transformable: Transformable) -> json::JsonValue {
	use json::JsonValue as J;
	match transformable {
		Transformable::No => J::Bool(false),
		Transformable::Free => J::Bool(true),
		Transformable::Sync => J::String("sync".to_string()),
		Transformable::Invert => J::String("invert".to_string()),
	}
}

/// The inverse of [`transform_value`]; `None` for anything that is not one of
/// the four.
fn parse_transform(value: &json::JsonValue) -> Option<Transformable> {
	match (value.as_bool(), value.as_str()) {
		(Some(true), _) => Some(Transformable::Free),
		(Some(false), _) => Some(Transformable::No),
		(_, Some("sync")) => Some(Transformable::Sync),
		(_, Some("invert")) => Some(Transformable::Invert),
		_ => None,
	}
}

/// A piece's pass grid as one string per cell row - a hex digit per cell, `.`
/// for a cell the source template left empty. Readable at a glance, which is
/// the point of it being in the text file at all.
fn pass_rows(piece: &SceneryPiece) -> Vec<json::JsonValue> {
	(0..piece.cells_h as usize)
		.map(|y| {
			let row: String = (0..piece.cells_w as usize)
				.map(|x| match piece.pass[y * piece.cells_w as usize + x] {
					PASS_EMPTY => '.',
					v => char::from_digit(v as u32, 16).unwrap_or('?'),
				})
				.collect();
			json::JsonValue::String(row)
		})
		.collect()
}

/// The inverse of [`pass_rows`], checked against the footprint it must fill.
fn parse_pass(rows: &[json::JsonValue], cells_w: usize, cells_h: usize) -> Result<Vec<u8>, String> {
	if rows.len() != cells_h {
		return Err(format!("{} pass rows, cells says {cells_h}", rows.len()));
	}
	let mut pass = Vec::with_capacity(cells_w * cells_h);
	for row in rows {
		let row = row.as_str().ok_or("a pass row is not a string")?;
		if row.chars().count() != cells_w {
			return Err(format!("a pass row is {} cells, cells says {cells_w}", row.chars().count()));
		}
		for c in row.chars() {
			pass.push(match c {
				'.' => PASS_EMPTY,
				c => c.to_digit(16).ok_or(format!("bad pass '{c}'"))? as u8,
			});
		}
	}
	Ok(pass)
}

/// One piece as a manifest entry, appending its two encoded planes to `data`
/// and recording their `[offset, length]`. The library manifest and the
/// single-piece [`SCN_MAGIC`] file are the same entry shape, so a piece that
/// round-trips through one round-trips through the other.
fn piece_entry(piece: &SceneryPiece, data: &mut Vec<u8>) -> json::JsonValue {
	use json::JsonValue as J;
	let (w, h) = (piece.sprite.width as usize, piece.sprite.height as usize);
	let body = encode_plane(&piece.sprite.body, w, h);
	let shade = encode_plane(&piece.sprite.shade, w, h);
	let body_at = data.len();
	data.extend_from_slice(&body);
	let shade_at = data.len();
	data.extend_from_slice(&shade);
	let pass = pass_rows(piece);
	// Relief is written only where it was authored: a piece whose relief is
	// inferred carries none of these keys and no third plane, so it encodes
	// exactly as it did before any of this existed.
	let mut relief: Vec<(String, J)> = Vec::new();
	if let Some(peak) = piece.peak {
		relief.push(("peak".to_string(), J::Number(peak as f64)));
	}
	if let Some(sunken) = piece.sunken {
		relief.push(("sunken".to_string(), J::Bool(sunken)));
	}
	if let Some(scarp) = piece.scarp {
		relief.push(("scarp".to_string(), J::Bool(scarp)));
	}
	// An authored height map rides inside the bundle, so a piece handed to
	// someone else as one `.scn` arrives with its relief intact. In a library it
	// is the separate `.hgt` that is authoritative - that is the file a user
	// edits - and this plane is what the bake wrote it from.
	if piece.height_authored() {
		let height = encode_plane(piece.height.as_ref().expect("authored"), w, h);
		let height_at = data.len();
		data.extend_from_slice(&height);
		relief.push(("height".to_string(), num_pair(height_at as u32, height.len() as u32)));
	}
	let mut entry = vec![
		("id".to_string(), J::String(piece.id.clone())),
		("name".to_string(), J::String(piece.name.clone())),
		("family".to_string(), J::String(piece.family.clone())),
		("transform".to_string(), transform_value(piece.transformable)),
		("cells".to_string(), num_pair(piece.cells_w, piece.cells_h)),
		("origin".to_string(), num_pair(piece.sprite.origin_x, piece.sprite.origin_y)),
		("size".to_string(), num_pair(piece.sprite.width, piece.sprite.height)),
		("pass".to_string(), J::Array(pass)),
		("body".to_string(), num_pair(body_at as u32, body.len() as u32)),
		("shade".to_string(), num_pair(shade_at as u32, shade.len() as u32)),
	];
	entry.extend(relief);
	J::Object(entry)
}

// ----- the shareable single-piece file ----------------------------------------

/// The `.scn` file's leading bytes. One piece, self-contained, so a cut-out can
/// be handed to someone else without shipping the library it lives in.
pub const SCN_MAGIC: &[u8; 8] = b"MMESCN1\n";

/// The extension a [`SCN_MAGIC`] file carries.
pub const SCN_EXT: &str = "scn";

/// Serialize one piece as a standalone `.scn`: the magic, a `u32` LE manifest
/// length, the manifest, then the plane blob its offsets index into. `pack`
/// rides along as a *hint* - the palette the art was drawn against - which an
/// import shows but never obeys, since the user picks the destination.
pub fn write_scn(piece: &SceneryPiece, pack: &str) -> Vec<u8> {
	use json::JsonValue as J;
	let mut data = Vec::new();
	let entry = piece_entry(piece, &mut data);
	let text = J::Object(vec![
		("version".to_string(), J::String(SCENERY_VERSION.to_string())),
		("pack".to_string(), J::String(pack.to_string())),
		("piece".to_string(), entry),
	])
	.to_pretty();
	let mut out = Vec::with_capacity(SCN_MAGIC.len() + 4 + text.len() + data.len());
	out.extend_from_slice(SCN_MAGIC);
	out.extend_from_slice(&(text.len() as u32).to_le_bytes());
	out.extend_from_slice(text.as_bytes());
	out.extend_from_slice(&data);
	out
}

/// Parse a `.scn`, returning the piece and the pack it was drawn for. The piece
/// comes back marked [`SceneryPiece::user`] - an imported object is the user's
/// by definition.
pub fn read_scn(bytes: &[u8]) -> Result<(SceneryPiece, String), String> {
	let head = bytes.get(..SCN_MAGIC.len()).ok_or("scn: too short to be a scenery file")?;
	if head != SCN_MAGIC {
		return Err("scn: not a scenery file (bad magic)".into());
	}
	let at = SCN_MAGIC.len();
	let len = bytes.get(at..at + 4).ok_or("scn: truncated header")?;
	let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
	let text = bytes.get(at + 4..at + 4 + len).ok_or("scn: truncated manifest")?;
	let text = std::str::from_utf8(text).map_err(|_| "scn: manifest is not utf-8".to_string())?;
	let data = &bytes[at + 4 + len..];
	let root = json::parse(text)?;
	let version = root.get("version").and_then(|v| v.as_str()).unwrap_or("");
	if version != SCENERY_VERSION {
		return Err(format!("scn: version '{version}', expected '{SCENERY_VERSION}'"));
	}
	let pack = root.get("pack").and_then(|v| v.as_str()).unwrap_or("").to_string();
	let entry = root.get("piece").ok_or("scn: missing piece")?;
	Ok((SceneryPiece { user: true, ..parse_piece(entry, data)? }, pack))
}

// ----- the authored relief file ------------------------------------------------

/// The `.hgt` file's leading bytes - one piece's **authored height map**, the
/// third of the three files a library piece is made of.
pub const HGT_MAGIC: &[u8; 8] = b"MMEHGT1\n";

/// The extension a [`HGT_MAGIC`] file carries.
pub const HGT_EXT: &str = "hgt";

/// Serialize an authored relief: the magic, the frame it was drawn in, then the
/// plane [`encode_plane`] would write.
///
/// Its own file rather than a third plane inside the `.scn`, because a relief is
/// **optional and separately authored**: a piece that never had one has no file,
/// a piece whose art is re-cut keeps its `.scn` and drops its `.hgt`, and the
/// bake can write height maps for a pack it did not otherwise touch. The frame
/// rides along so a plane that no longer fits its sprite is caught at load
/// rather than silently mis-read a row at a time.
pub fn write_hgt(height: &[u8], width: u16, sprite_height: u16) -> Vec<u8> {
	let (w, h) = (width as usize, sprite_height as usize);
	assert_eq!(height.len(), w * h, "write_hgt: plane is not width * height");
	let mut out = Vec::with_capacity(HGT_MAGIC.len() + 4 + height.len() / 2);
	out.extend_from_slice(HGT_MAGIC);
	out.extend_from_slice(&width.to_le_bytes());
	out.extend_from_slice(&sprite_height.to_le_bytes());
	out.extend_from_slice(&encode_plane(height, w, h));
	out
}

/// Parse a `.hgt`, returning the plane and the frame it was drawn in. Errors
/// rather than panics - it parses bytes off disk, and a hand-edited or stale
/// height map is a thing that happens.
pub fn read_hgt(bytes: &[u8]) -> Result<(Vec<u8>, u16, u16), String> {
	let head = bytes.get(..HGT_MAGIC.len()).ok_or("hgt: too short to be a height map")?;
	if head != HGT_MAGIC {
		return Err("hgt: not a height map (bad magic)".into());
	}
	let at = HGT_MAGIC.len();
	let frame = bytes.get(at..at + 4).ok_or("hgt: truncated header")?;
	let width = u16::from_le_bytes([frame[0], frame[1]]);
	let height = u16::from_le_bytes([frame[2], frame[3]]);
	let plane = decode_plane(&bytes[at + 4..], width as usize, height as usize)?;
	Ok((plane, width, height))
}

/// A height map **as a picture**: each pixel's elevation against the peak the
/// piece stands at, so white is the top of the object and a grey ramp is used
/// however low the object actually stands.
///
/// Scaled per object, and to `peak` rather than to the tallest pixel there
/// happens to be. Per object because a relief is in map pixels of elevation and
/// a 30px boulder's whole relief is fifteen of them - against a fixed scale it
/// is fifteen shades of black, which is not a picture anybody can judge or
/// paint on. To the **peak** because this picture is the one somebody paints
/// on: [`height_from_grey`] reads it back through the same number, so what you
/// paint is what the piece stands at, and a stretch to whatever the tallest
/// pixel happened to be would quietly raise every object it round-tripped.
///
/// `0` stays `0`: outside the object there is no elevation, only ground.
pub fn height_to_grey(height: &[u8], peak: u8) -> Vec<u8> {
	let peak = peak.max(1) as u32;
	height.iter().map(|&h| ((h as u32).min(peak) * 255 / peak) as u8).collect()
}

/// The inverse: a **painted** height map read back into elevations, in the
/// sprite's frame.
///
/// `grey` is one byte per pixel over a `w` x `h` image, which must be either the
/// sprite's own box or the piece's whole footprint in cells - the two frames
/// [`height_to_grey`]'s picture is ever seen in - and `None` otherwise, because
/// a height map that does not line up with the art is not a height map for it.
///
/// Three things it settles, none of which the image can say for itself:
///
/// * **Where the object is** is the body plane's business, not the painter's. A
///   grey pixel off the object is dropped and a body pixel left black still
///   stands at 1, so the relief cannot disagree with the silhouette about what
///   is even there.
/// * **How high white is** is `peak` - the `Stands:` choice. That is what keeps
///   a picture stretched to its own white from making every object equally tall.
/// * **Nothing exceeds the peak**, so two pieces stay comparable in the units
///   [`SceneryBlend::Higher`] compares them in.
pub fn height_from_grey(
	grey: &[u8],
	w: usize,
	h: usize,
	sprite: &Sprite,
	cells_w: u16,
	cells_h: u16,
	peak: u8,
) -> Option<Vec<u8>> {
	let (sw, sh) = (sprite.width as usize, sprite.height as usize);
	if grey.len() < w * h || sw == 0 || sh == 0 {
		return None;
	}
	// The sprite's own box, or the footprint the sprite was cropped out of - in
	// which case the crop origin says which part of it is the object.
	let (ox, oy) = match (w, h) {
		(iw, ih) if (iw, ih) == (sw, sh) => (0, 0),
		(iw, ih) if (iw, ih) == (cells_w as usize * CELL_PX, cells_h as usize * CELL_PX) => {
			(sprite.origin_x as usize, sprite.origin_y as usize)
		}
		_ => return None,
	};
	let peak = peak.max(1) as u32;
	let mut out = vec![0u8; sw * sh];
	for y in 0..sh {
		for x in 0..sw {
			let i = y * sw + x;
			if sprite.body[i] == 0 {
				continue;
			}
			let (gx, gy) = (x + ox, y + oy);
			let value = if gx < w && gy < h { grey[gy * w + gx] as u32 } else { 0 };
			out[i] = ((value * peak + 127) / 255).clamp(1, peak) as u8;
		}
	}
	Some(out)
}

// ----- authoring a piece from an image ----------------------------------------

/// How an authored image's alpha channel splits into the two planes - **the**
/// rule, shared by every door a PNG comes in through (the New Scenery dialog
/// and the offline bake's hand-cut path), so one image cuts the same way
/// whichever one it came through.
///
/// A cut-out is three things - the object, the ground it shades, and the
/// nothing around it - and the alpha channel names all three with no mask
/// channel and no naming convention:
///
/// | alpha | means |
/// |---|---|
/// | `0` | nothing at all: not the object, not its shadow. Masked out. |
/// | [`SHADOW_BAND`] (~50%) | the object's cast shadow: flat black at [`SHADOW_ALPHA`]. |
/// | anything else | the object's own ink, at the nearest palette colour. |
///
/// Only *fully* transparent is nothing. A pixel the artist left at 10% or 90%
/// alpha is still a pixel they painted, so it keeps its colour rather than
/// being silently eaten - the alpha channel is a three-way switch here, not an
/// opacity the renderer honours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageBand {
	/// Fully transparent: no ink, no shadow.
	Clear,
	/// Around half transparent: the object's cast shadow.
	Shadow,
	/// Everything else: the object's own ink.
	Body,
}

/// The alpha window that reads as "about half transparent" - 37.6% to 62.7%.
///
/// Wide enough that 126, 127, 128 and a paint program's idea of "50%" all land
/// in it, narrow enough that art at a quarter or three quarters opacity is
/// plainly ink. The band is a switch, not a measurement: every pixel inside it
/// becomes the same [`SHADOW_ALPHA`], because the shadow plane is one flat
/// alpha per object by design (see [`CutOpts::alpha`]).
pub const SHADOW_BAND: std::ops::RangeInclusive<u8> = 96..=160;

/// What a [`ImageBand::Shadow`] pixel is recorded at: half-transparent black.
pub const SHADOW_ALPHA: u8 = 128;

/// Which plane a pixel of this alpha joins - see [`ImageBand`].
pub fn band_of(alpha: u8) -> ImageBand {
	match alpha {
		0 => ImageBand::Clear,
		a if SHADOW_BAND.contains(&a) => ImageBand::Shadow,
		_ => ImageBand::Body,
	}
}

/// What an imported image cannot read off its own pixels.
///
/// The alpha rule ([`ImageBand`]) settles every pixel, so all that is left is
/// the footprint's verdict on the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterOpts {
	/// The pass value the footprint's covered cells impose (0 land / 1 water /
	/// 2 shore / 3 blocked). An object blocks by default.
	pub pass: u8,
}

impl Default for RasterOpts {
	fn default() -> Self {
		Self { pass: 3 }
	}
}

/// One map cell, in pixels - the grid an imported image's footprint is cut on.
const CELL_PX: usize = 64;

/// How much of a cell an imported object's body must cover before that cell
/// blocks. The same eighth the placement rule uses, so what the picker's
/// footprint promises is what the map enforces.
const IMPORT_PASS_COVERAGE: usize = CELL_PX * CELL_PX / 8;

/// The palette index closest to `rgb` that a cut-out may actually use: never
/// `0` (which means "nothing here" in a body plane) and never a slot the game
/// colour-cycles, so an object does not shimmer where the art it was cut beside
/// stays still. Perceptually-weighted squared distance, as the palette
/// converter uses.
fn nearest_body_index(palette: &[u8], rgb: [u8; 3]) -> u8 {
	let usable = |i: u8| i != 0 && !ANIMATED_SLOTS.contains(&i) && !WATER_SLOTS.contains(&i);
	(1..=255u8)
		.filter(|&i| usable(i))
		.min_by_key(|&i| {
			let c = slot_rgb(palette, i);
			let (dr, dg, db) = (c[0] as i64 - rgb[0] as i64, c[1] as i64 - rgb[1] as i64, c[2] as i64 - rgb[2] as i64);
			// Redmean: cheap, and closer to how the eye ranks two near misses.
			let rm = (c[0] as i64 + rgb[0] as i64) / 2;
			((512 + rm) * dr * dr) + 1024 * dg * dg + ((767 - rm) * db * db)
		})
		.unwrap_or(1)
}

/// The two planes an authored image cuts into, uncropped and row-major over the
/// whole image: body palette indices and shade alphas. The alpha rule
/// ([`ImageBand`]) settles every pixel; the colours are the image's own,
/// clamped to the nearest palette ink a cut-out may use.
fn image_planes(rgba: &[u8], width: usize, height: usize, palette: &[u8]) -> (Vec<u8>, Vec<u8>) {
	let mut body = vec![0u8; width * height];
	let mut shade = vec![0u8; width * height];
	// One cache entry per distinct colour: an image is a few dozen colours over
	// a few hundred thousand pixels, and the search is 200-odd candidates.
	let mut seen: std::collections::HashMap<[u8; 3], u8> = std::collections::HashMap::new();
	for i in 0..width * height {
		let px = &rgba[i * 4..i * 4 + 4];
		match band_of(px[3]) {
			ImageBand::Clear => {}
			ImageBand::Shadow => shade[i] = SHADOW_ALPHA,
			ImageBand::Body => {
				let rgb = [px[0], px[1], px[2]];
				body[i] = *seen.entry(rgb).or_insert_with(|| nearest_body_index(palette, rgb));
			}
		}
	}
	(body, shade)
}

/// Cut a placeable sprite straight out of an RGBA image, by the alpha rule
/// ([`ImageBand`]) alone: the image *is* the object.
///
/// The result is cropped to what survived, so the sprite's origin says where
/// the object sat in the image. Empty when the image is entirely transparent.
///
/// This is the whole of the authoring path a `.png` takes - there is no ground
/// flood to fool and no shadow ink set to infer, because the artist already
/// answered both questions with the alpha channel. [`cut`] is the other path:
/// art that was never authored as a cut-out, where both have to be guessed.
pub fn cut_image(rgba: &[u8], width: usize, height: usize, palette: &[u8]) -> Sprite {
	if width == 0 || height == 0 || rgba.len() < width * height * 4 {
		return Sprite::default();
	}
	let (body, shade) = image_planes(rgba, width, height, palette);
	crop(body, shade, width, height)
}

/// Rasterize an RGBA image into a placeable piece, the way a new tile is
/// rasterized: every kept pixel lands on a real palette index.
///
/// The alpha channel decides which plane a pixel joins ([`ImageBand`]); the
/// result is cropped to what survived, and the footprint is the whole *image*
/// in cells - the crop moves the sprite inside that box, exactly as a cut does,
/// so an object keeps its position when it is re-imported. Returns `None` when
/// the image is entirely transparent.
pub fn rasterize(
	rgba: &[u8],
	width: usize,
	height: usize,
	palette: &[u8],
	opts: &RasterOpts,
) -> Option<(Sprite, Vec<u8>, u16, u16)> {
	if width == 0 || height == 0 || rgba.len() < width * height * 4 {
		return None;
	}
	let (body, shade) = image_planes(rgba, width, height, palette);
	let sprite = crop(body.clone(), shade, width, height);
	if sprite.is_empty() {
		return None;
	}

	// The footprint, and which of its cells the body covers enough to block.
	let (cells_w, cells_h) = (width.div_ceil(CELL_PX), height.div_ceil(CELL_PX));
	let mut pass = Vec::with_capacity(cells_w * cells_h);
	for cy in 0..cells_h {
		for cx in 0..cells_w {
			let mut covered = 0usize;
			for y in cy * CELL_PX..((cy + 1) * CELL_PX).min(height) {
				for x in cx * CELL_PX..((cx + 1) * CELL_PX).min(width) {
					covered += usize::from(body[y * width + x] != 0);
				}
			}
			pass.push(if covered >= IMPORT_PASS_COVERAGE { opts.pass.min(3) } else { PASS_EMPTY });
		}
	}
	Some((sprite, pass, cells_w as u16, cells_h as u16))
}

fn parse_piece(value: &json::JsonValue, data: &[u8]) -> Result<SceneryPiece, String> {
	let id = value.get("id").and_then(|v| v.as_str()).ok_or("scenery: a piece has no id")?.to_string();
	let name = value.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string();
	// Both fields are written by the bake; a library from before they existed
	// falls back to what they are prefilled from.
	let family =
		value.get("family").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| piece_family(&name));
	// `transform` is what a piece's own files say; `transformable` is the name
	// `tiles.props.json` gives the identical four values.
	let transformable = match value.get("transform").or_else(|| value.get("transformable")) {
		None => Transformable::No,
		Some(v) => {
			parse_transform(v).ok_or_else(|| format!("scenery {id}: bad transform (true|false|\"sync\"|\"invert\")"))?
		}
	};
	// Absent = inferred, which is what every shipped piece does; present = the
	// author overruled the guess (`SceneryPiece::height_opts`).
	let peak = match value.get("peak") {
		None => None,
		Some(v) => match v.as_f64().filter(|f| (1.0..=255.0).contains(f)) {
			Some(f) => Some(f as u8),
			None => return Err(format!("scenery {id}: bad peak (1..=255)")),
		},
	};
	let sunken = match value.get("sunken") {
		None => None,
		Some(v) => match v.as_bool() {
			Some(b) => Some(b),
			None => return Err(format!("scenery {id}: bad sunken (true|false)")),
		},
	};
	let scarp = match value.get("scarp") {
		None => None,
		Some(v) => match v.as_bool() {
			Some(b) => Some(b),
			None => return Err(format!("scenery {id}: bad scarp (true|false)")),
		},
	};
	let pair = |key: &str| -> Result<(usize, usize), String> {
		let a = value.get(key).and_then(|v| v.as_array()).ok_or(format!("scenery {id}: missing {key}"))?;
		if a.len() != 2 {
			return Err(format!("scenery {id}: {key} is not a pair"));
		}
		let n = |v: &json::JsonValue| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as usize);
		Ok((n(&a[0]).ok_or(format!("scenery {id}: bad {key}"))?, n(&a[1]).ok_or(format!("scenery {id}: bad {key}"))?))
	};
	let (cells_w, cells_h) = pair("cells")?;
	let (origin_x, origin_y) = pair("origin")?;
	let (width, height) = pair("size")?;
	let (body_at, body_len) = pair("body")?;
	let (shade_at, shade_len) = pair("shade")?;
	let slice = |at: usize, len: usize| -> Result<&[u8], String> {
		data.get(at..at + len).ok_or(format!("scenery {id}: plane [{at}..{}] is past the data", at + len))
	};
	let sprite = Sprite {
		width: width as u16,
		height: height as u16,
		origin_x: origin_x as u16,
		origin_y: origin_y as u16,
		body: decode_plane(slice(body_at, body_len)?, width, height)?,
		shade: decode_plane(slice(shade_at, shade_len)?, width, height)?,
	};
	// The bundled relief, present only on a piece somebody drew one for.
	let relief = match value.get("height") {
		None => None,
		Some(_) => {
			let (at, len) = pair("height")?;
			Some(decode_plane(slice(at, len)?, width, height)?)
		}
	};
	let rows = value.get("pass").and_then(|v| v.as_array()).ok_or(format!("scenery {id}: missing pass"))?;
	let pass = parse_pass(rows, cells_w, cells_h).map_err(|e| format!("scenery {id}: {e}"))?;
	Ok(SceneryPiece {
		id,
		name,
		family,
		transformable,
		peak,
		sunken,
		scarp,
		height: relief,
		cells_w: cells_w as u16,
		cells_h: cells_h as u16,
		pass,
		sprite,
		user: false,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::palette::set_slot_rgb;

	/// A 8x8 box: a 4x4 solid body block in the middle, one enclosed ground
	/// pixel inside it, a shadow pixel outside it, ground everywhere else.
	fn fixture() -> (Vec<Option<u8>>, GroundInk, ShadowInk) {
		let mut ground = GroundInk { is_ground: [false; 256] };
		ground.is_ground[10] = true;
		let mut shadow = ShadowInk::new();
		shadow.insert(20);
		let mut src = vec![Some(10u8); 64];
		for y in 2..6 {
			for x in 2..6 {
				src[y * 8 + x] = Some(30);
			}
		}
		src[3 * 8 + 3] = Some(10); // ground the body encloses
		src[6 * 8 + 1] = Some(20); // shadow on open ground
		(src, ground, shadow)
	}

	#[test]
	fn enclosed_ground_stays_with_the_object() {
		let (src, ground, shadow) = fixture();
		let sprite = cut(&src, 8, 8, &ground, &shadow, &CutOpts { close: 0, alpha: 100 });
		// Cropped to the body block plus the lone shadow pixel below-left of it.
		assert_eq!((sprite.origin_x, sprite.origin_y), (1, 2));
		assert_eq!((sprite.width, sprite.height), (5, 5));
		let at = |x: usize, y: usize| y * sprite.width as usize + x;
		// The enclosed ground pixel kept its own index and is opaque.
		assert_eq!(sprite.body[at(2, 1)], 10, "enclosed ground stays");
		assert_eq!(sprite.shade[at(2, 1)], 0);
		// The body is body.
		assert_eq!(sprite.body[at(1, 0)], 30);
		// The shadow pixel shades, and carries no body ink.
		assert_eq!(sprite.shade[at(0, 4)], 100, "open-ground shadow shades");
		assert_eq!(sprite.body[at(0, 4)], 0);
		// Open ground is gone.
		assert_eq!(sprite.body[at(0, 0)], 0);
		assert_eq!(sprite.shade[at(0, 0)], 0);
	}

	// ----- the inferred height field ------------------------------------------

	/// A grey ramp: ink `i` has brightness `i`, so a test can name a luma.
	fn grey_palette() -> [u8; 256] {
		let mut palette = vec![0u8; 768];
		for i in 0..256usize {
			set_slot_rgb(&mut palette, i as u8, [i as u8, i as u8, i as u8]);
		}
		brightness_table(&palette)
	}

	/// A `w` x `h` sprite whose body is `ink` everywhere.
	fn slab(w: u16, h: u16, ink: u8) -> Sprite {
		let n = w as usize * h as usize;
		Sprite { width: w, height: h, origin_x: 0, origin_y: 0, body: vec![ink; n], shade: vec![0; n] }
	}

	/// The dome: a solid block stands tallest where it is deepest inside its own
	/// silhouette, and nothing outside the body has a height at all.
	#[test]
	fn a_height_field_domes_towards_the_core() {
		let sprite = slab(21, 21, 100);
		let field = sprite.height_field(
			&grey_palette(),
			&HeightOpts { peak: 200, sunken: false, pyramid: false, scarp: false, rim: &[], foot: &[] },
		);
		let at = |x: usize, y: usize| field[y * 21 + x];
		assert!(at(10, 10) > at(4, 10), "the core stands over the flank: {} vs {}", at(10, 10), at(4, 10));
		assert!(at(4, 10) > at(0, 10), "and the flank over the rim: {} vs {}", at(4, 10), at(0, 10));
		assert!(field.iter().all(|&h| h <= 200), "nothing passes the peak");
		assert!(field.iter().all(|&h| h >= 1), "and no body pixel reads as bare ground");
	}

	/// **Sunken is a ring, not an upside-down dome.** A crater's outline is
	/// ground level, it rises to a raised rim a little way in, and only then
	/// does it fall away into the hole - inverting the dome instead put the
	/// highest point on the outermost pixel of the ejecta and read as one smooth
	/// funnel.
	#[test]
	fn a_sunken_piece_rises_to_a_rim_and_falls_inside_it() {
		// 41x41, so the deepest point is 20 pixels in and the rim (35%) lands
		// around 7 - far enough from both ends to be told apart from them.
		let sprite = slab(41, 41, 100);
		let field = sprite.height_field(
			&grey_palette(),
			&HeightOpts { peak: 200, sunken: true, pyramid: false, scarp: false, rim: &[], foot: &[] },
		);
		let at = |x: usize| field[20 * 41 + x]; // the middle row, outside in
		let rim = (0..=20).max_by_key(|&x| at(x)).expect("a tallest pixel on the row");
		assert!((4..=11).contains(&rim), "the rim stands about a third of the way in, not at {rim}");
		assert!(at(0) < at(rim), "the outline is ground level: {} vs {}", at(0), at(rim));
		assert!(at(20) < at(rim), "and the bowl is below the rim: {} vs {}", at(20), at(rim));
		// Monotone on both sides of it - a rim, not a ridge with dips.
		assert!((1..rim).all(|x| at(x) <= at(x + 1)), "it rises to the rim");
		assert!((rim..20).all(|x| at(x) >= at(x + 1)), "and falls away inside it");
	}

	/// **A traced rim beats the 35% guess.** `RIM_AT` is one fraction for every
	/// crater there is; a curve drawn on the art says where this one's crest
	/// actually runs, and the ring lands on it however lopsided it is.
	#[test]
	fn a_drawn_rim_puts_the_ring_where_it_was_drawn() {
		// A 61x61 slab with a square crest traced 20 px in on the left and 6 px in
		// on the right - a ring no single fraction of the depth could describe.
		let (n, sprite) = (61usize, slab(61, 61, 100));
		let (left, right, top, bottom) = (20usize, 54usize, 20usize, 40usize);
		let mut rim = vec![false; n * n];
		for y in top..=bottom {
			rim[y * n + left] = true;
			rim[y * n + right] = true;
		}
		for x in left..=right {
			rim[top * n + x] = true;
			rim[bottom * n + x] = true;
		}
		let field = sprite.height_field(
			&grey_palette(),
			&HeightOpts { peak: 200, sunken: true, pyramid: false, scarp: false, rim: &rim, foot: &[] },
		);
		let at = |x: usize| field[30 * n + x]; // the middle row, left to right
		// Each side's tallest pixel is that side's drawn crest - and they tie,
		// which is the point: one fraction of the depth could only put them at the
		// same distance in.
		assert_eq!((0..n / 2).max_by_key(|&x| at(x)), Some(left), "the left crest is where the line is");
		assert_eq!((n / 2..n).max_by_key(|&x| at(x)), Some(right), "and so is the right one");
		assert_eq!((at(left), at(right)), (200, 200), "both stand at the peak, which is what a crest is");
		assert!(at(0) < at(left) && at(n - 1) < at(right), "both flanks climb out of the ground");
		// The bowl reaches the ground, rather than 40% of however brightly its
		// floor was painted - the luma has no say inside a drawn rim.
		let floor = (left + 1..right).min_by_key(|&x| at(x)).expect("a lowest pixel in the bowl");
		assert_eq!(at(floor), 1, "the bowl bottoms out at ground level");
		assert!(at(35) < at(left) && at(35) > 0, "and it is still the object all the way down");
		// Each flank slopes over its own width, not over one shared fraction.
		assert!((1..left).all(|x| at(x) <= at(x + 1)), "the long flank rises all the way in");
		assert!((right..n - 1).all(|x| at(x) >= at(x + 1)), "and the short one falls all the way out");
	}

	/// **A pyramid is built, not read off the art.** Its four faces are flat
	/// planes at equal slope meeting at an apex - a shape known before you look -
	/// so neither the dome nor the luma gets a say, and the lit face stands
	/// exactly as high as the dark one.
	#[test]
	fn a_pyramid_is_four_flat_faces_and_an_apex() {
		// Lit hard along one flank, and quartered by a gap the way DESERT's
		// pyramids are quartered by their causeway: two things that wreck the
		// inference and neither of which a built shape can see.
		let (n, mut sprite) = (41usize, slab(41, 41, 40));
		for y in 0..n {
			for x in 0..n {
				sprite.body[y * n + x] = match () {
					_ if x == 20 || y == 20 => 0, // the causeway - not body at all
					_ if x < 10 => 240,           // ...and a sunlit western flank
					_ => 40,
				};
			}
		}
		let opts = HeightOpts { peak: 200, sunken: false, pyramid: true, scarp: false, rim: &[], foot: &[] };
		let field = sprite.height_field(&grey_palette(), &opts);
		let at = |x: usize, y: usize| field[y * n + x] as i32;
		// The apex pixel itself is causeway, so the nearest body to it is the
		// tallest thing there is - and it is within a twentieth of the full peak,
		// rather than the ground the silhouette beside a gap would have put it at.
		assert_eq!(field.iter().copied().max(), Some(at(19, 19) as u8), "the apex is the tallest point");
		assert!(at(19, 19) >= 190, "and it stands at the peak the shape implies: {}", at(19, 19));
		assert_eq!(at(5, 20 - 5), at(n - 6, 20 + 5), "the lit face and the dark one stand equal");
		assert_eq!(at(10, 3), at(3, 10), "and so do two faces the causeway cut apart");
		// Straight faces: along a diagonal from the corner to the apex the rise is
		// constant - peak over the ridge, 200/20 - which is what makes it a plane
		// rather than a dome. Past the skirt, which bends the first few pixels down
		// to the ground on purpose.
		let rise: Vec<i32> = (4..18).map(|k| at(k + 1, k + 1) - at(k, k)).collect();
		assert!(rise.iter().all(|&d| (9..=11).contains(&d)), "a flat face rises evenly: {rise:?}");
		assert!(field.iter().enumerate().all(|(i, &h)| sprite.body[i] == 0 || h >= 1), "and it is all object");
	}

	/// An open curve has no inside, so there is no bowl to put anywhere and the
	/// [`RIM_AT`] guess is worth more than half a rim.
	#[test]
	fn an_unclosed_rim_falls_back_to_the_guess() {
		let (n, sprite) = (41usize, slab(41, 41, 100));
		let mut rim = vec![false; n * n];
		for y in 10..30 {
			rim[y * n + 10] = true; // one stroke, enclosing nothing
		}
		let opts = HeightOpts { peak: 200, sunken: true, pyramid: false, scarp: false, rim: &rim, foot: &[] };
		let guess = HeightOpts { rim: &[], ..opts };
		assert_eq!(
			sprite.height_field(&grey_palette(), &opts),
			sprite.height_field(&grey_palette(), &guess),
			"an open curve is no curve"
		);
	}

	/// **A scarp is a step in the land, not a ridge on it.** Read as a dome, a
	/// wall is highest down its middle and falls back to ground on both sides -
	/// so the brow an object is meant to stand on sinks away under it. The
	/// shape has to run ground-to-peak straight across the band instead.
	/// A wall running north-east to south-west inside a frame of open ground, at
	/// a flat ink so nothing but the shape is talking. `N` wide, the band `BAND`
	/// pixels thick about the anti-diagonal.
	fn wall(n: usize, band: usize) -> Sprite {
		let mut sprite = slab(n as u16, n as u16, 0);
		sprite.body = vec![0; n * n];
		for y in 0..n {
			for x in 0..n {
				// |x + y - n| small: the anti-diagonal, whose two long edges face
				// north-west and south-east - the flanks the light tells apart.
				if (x as isize + y as isize - n as isize).unsigned_abs() <= band {
					sprite.body[y * n + x] = 120;
				}
			}
		}
		sprite
	}

	#[test]
	fn a_scarp_climbs_from_its_foot_to_its_brow() {
		const N: usize = 61;
		let sprite = wall(N, 20);
		let opts = HeightOpts { peak: 200, sunken: false, pyramid: false, scarp: true, rim: &[], foot: &[] };
		let field = sprite.height_field(&grey_palette(), &opts);
		let at = |x: usize, y: usize| field[y * N + x];
		// Straight across the band through its middle, north-west to south-east.
		// The light comes over the north-west flank, so that edge is the brow and
		// the south-east edge is the foot the wall shadows. `|x + y - N| <= 20`
		// along `x == y` is `k` in `-9..=10` off the centre pixel.
		let mid = N / 2;
		let across: Vec<u8> = (0..20).map(|k| at(mid - 9 + k, mid - 9 + k)).collect();
		let (brow, foot) = (across[0], across[19]);
		assert!(brow > foot, "the brow stands over the foot: {brow} vs {foot}");
		assert_eq!(foot, 1, "and the foot is ground level: {across:?}");
		// It climbs the whole way rather than doming and coming back, so the
		// highest pixel of the crossing is its north-west end.
		assert_eq!(across.iter().max(), Some(&brow), "the brow is the peak: {across:?}");
	}

	/// The wall is the shape; **the ridge it replaces is what a cliff was
	/// getting**, and the two have to differ at the one pixel that matters.
	#[test]
	fn a_scarp_keeps_the_brow_a_dome_sands_away() {
		const N: usize = 61;
		let sprite = wall(N, 20);
		let plain = HeightOpts { peak: 200, sunken: false, pyramid: false, scarp: false, rim: &[], foot: &[] };
		let dome = sprite.height_field(&grey_palette(), &plain);
		let wall_field = sprite.height_field(&grey_palette(), &HeightOpts { scarp: true, ..plain });
		let mid = N / 2;
		let brow = (mid - 9) * N + (mid - 9); // the north-west lip of the band
		assert_eq!(dome[brow], 1, "the dome sands the brow down to the ground");
		assert!(wall_field[brow] > dome[brow], "the wall does not: {} vs {}", wall_field[brow], dome[brow]);
		// One shape to a piece: sunken outranks it.
		let sunk = HeightOpts { scarp: true, sunken: true, ..plain };
		assert_eq!(
			sprite.height_field(&grey_palette(), &sunk),
			sprite.height_field(&grey_palette(), &HeightOpts { scarp: false, ..sunk }),
			"a bowl is not also a wall"
		);
	}

	/// **A traced brow puts the peak where somebody drew it.** The cliff case is
	/// a loop of wall around open ground: the curve runs along the *inner* edge
	/// of the band and encloses nothing the piece owns, which is exactly the
	/// curve [`rim_dome`] refuses and [`scarp_rim`] is for.
	#[test]
	fn a_traced_brow_stands_where_the_line_was_drawn() {
		const N: usize = 61;
		const MID: isize = 30;
		// A square ring of wall: body between Chebyshev radius 10 and 20, open
		// ground inside it and out.
		let cheb = |i: usize| {
			let (x, y) = ((i % N) as isize - MID, (i / N) as isize - MID);
			x.abs().max(y.abs())
		};
		let mut sprite = slab(N as u16, N as u16, 0);
		for i in 0..N * N {
			sprite.body[i] = if (10..=20).contains(&cheb(i)) { 120 } else { 0 };
		}
		// ...traced along the ring's inner edge, which is a closed curve.
		let rim: Vec<bool> = (0..N * N).map(|i| cheb(i) == 10).collect();
		let plain = HeightOpts { peak: 200, sunken: false, pyramid: false, scarp: true, rim: &[], foot: &[] };
		let drawn = sprite.height_field(&grey_palette(), &HeightOpts { rim: &rim, ..plain });
		let at = |x: isize, y: isize| drawn[(y + MID) as usize * N + (x + MID) as usize];
		// Due west, straight across the band: the outer edge is the foot and the
		// traced inner edge is the brow.
		assert_eq!(at(-20, 0), 1, "the outer edge is ground level");
		assert_eq!(at(-10, 0), 200, "the drawn line is the peak, flat at it");
		let across: Vec<u8> = (0..=10).map(|k| at(-20 + k, 0)).collect();
		assert!(across.windows(2).all(|w| w[1] >= w[0]), "it climbs to the line: {across:?}");
		// The luma is silenced on the crest, so the line reads a flat peak rather
		// than 40% of however brightly the art painted it.
		assert!((-10..=10).all(|y| at(-10, y) == 200), "the whole traced line stands at the peak");
		// **An open arc still draws the crest**, and only the sides fall back to
		// the light - a cliff's brow is a line along a band, and it closes into a
		// loop only when the band happens to.
		let open: Vec<bool> = (0..N * N).map(|i| cheb(i) == 10 && (i / N) < 40).collect();
		let arc = sprite.height_field(&grey_palette(), &HeightOpts { rim: &open, ..plain });
		let arc_at = |x: isize, y: isize| arc[(y + MID) as usize * N + (x + MID) as usize];
		assert_eq!(arc_at(-10, 0), 200, "the drawn stretch is still the peak");
		assert!(arc_at(-10, 0) > arc_at(-20, 0), "and the wall still climbs to it");
		// ...but where nobody drew, the light is back in charge: the south stretch
		// of the inner edge is the peak where the loop was traced over it and is
		// not where the arc stopped short.
		assert_eq!(at(0, 10), 200, "the traced loop crests the south inner edge");
		assert!(arc_at(0, 10) < 200, "the arc left that stretch to the light: {}", arc_at(0, 10));
	}

	/// **Two lines say which way the land steps, and the band's own edges say how
	/// high.** Red on the inner side, green on the outer, and every side of the
	/// ring reads the same way - which the light alone cannot do, since it comes
	/// from one direction and a cliff loops around.
	#[test]
	fn two_traced_lines_orient_the_whole_wall() {
		const N: usize = 61;
		const MID: isize = 30;
		let cheb = |i: usize| {
			let (x, y) = ((i % N) as isize - MID, (i / N) as isize - MID);
			x.abs().max(y.abs())
		};
		let mut sprite = slab(N as u16, N as u16, 0);
		for i in 0..N * N {
			sprite.body[i] = if (10..=20).contains(&cheb(i)) { 120 } else { 0 };
		}
		let rim: Vec<bool> = (0..N * N).map(|i| cheb(i) == 5).collect();
		let foot: Vec<bool> = (0..N * N).map(|i| cheb(i) == 25).collect();
		let opts = HeightOpts { peak: 200, sunken: false, pyramid: false, scarp: true, rim: &rim, foot: &foot };
		let field = sprite.height_field(&grey_palette(), &opts);
		let at = |x: isize, y: isize| field[(y + MID) as usize * N + (x + MID) as usize];
		for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
			// Ground at the outer edge of the band, highest at the inner - the
			// silhouette's own edges, not where the lines happen to run. The peak is
			// not a flat 200: the luma keeps its 40% all the way across, because a
			// direction is not a height and has nothing to contradict.
			let across: Vec<u8> = (0..=10).map(|k| at((20 - k) * dx, (20 - k) * dy)).collect();
			assert_eq!(across[0], 1, "the outer edge is ground at ({dx},{dy})");
			assert_eq!(across.iter().max(), across.last(), "the inner edge is the high one at ({dx},{dy})");
			assert!(across.windows(2).all(|w| w[1] >= w[0]), "it climbs outward-in at ({dx},{dy}): {across:?}");
		}
	}

	/// **A loose line is as good as a careful one.** The lines are read for their
	/// direction alone, so sliding either of them about inside its own ground -
	/// which is what a freehand stroke along a ragged band does - may not move the
	/// relief at all. This is the property the contour reading did not have.
	#[test]
	fn a_wobble_in_a_traced_line_does_not_move_the_relief() {
		const N: usize = 61;
		const MID: isize = 30;
		let cheb = |i: usize| {
			let (x, y) = ((i % N) as isize - MID, (i / N) as isize - MID);
			x.abs().max(y.abs())
		};
		let mut sprite = slab(N as u16, N as u16, 0);
		for i in 0..N * N {
			sprite.body[i] = if (10..=20).contains(&cheb(i)) { 120 } else { 0 };
		}
		let plain = HeightOpts { peak: 200, sunken: false, pyramid: false, scarp: true, rim: &[], foot: &[] };
		let field = |r: isize, g: isize| {
			let rim: Vec<bool> = (0..N * N).map(|i| cheb(i) == r).collect();
			let foot: Vec<bool> = (0..N * N).map(|i| cheb(i) == g).collect();
			sprite.height_field(&grey_palette(), &HeightOpts { rim: &rim, foot: &foot, ..plain })
		};
		// Both lines moved several pixels, each staying on its own side of the
		// wall. Same answer: only which ground is which was ever read off them.
		assert_eq!(field(5, 25), field(2, 28), "the relief follows the band, not the stroke");
		// ...and swapping the two flips the wall, which is the one thing a
		// direction is entitled to change.
		let (up, down) = (field(5, 25), field(25, 5));
		assert_ne!(up, down, "red and green are not interchangeable");
		let at = |f: &Vec<u8>, x: isize, y: isize| f[(y + MID) as usize * N + (x + MID) as usize];
		assert!(at(&down, 20, 0) > at(&down, 10, 0), "swapped, the outer edge is the high one");
		assert_eq!(at(&down, 10, 0), 1, "and the inner edge is the ground");
	}

	/// **A drift is not as tall as the ground it covers is wide.** [`default_peak`]
	/// reads height off footprint, which is right within a family and wrong across
	/// them: unscaled, the widest dune in the packs stands over the median
	/// mountain and `SceneryBlend::Higher` resolves every such overlap backwards.
	#[test]
	fn low_families_stand_under_the_landmarks() {
		// The widest dune and rough in the shipped art against a middling mountain.
		let (widest_low, median_tall) = (93u8, 47u8);
		for family in ["dune", "rough", "rouge", "meadow"] {
			let low = family_peak(family, widest_low);
			assert!(low < median_tall, "{family} at {low} still tops a mountain at {median_tall}");
		}
		// A landmark is left exactly as the footprint found it.
		for family in ["mountain", "mountain-d", "trees", "crater", "pyramid", "cliff"] {
			assert_eq!(family_peak(family, 80), 80, "{family} is not ground cover");
		}
		// Monotone within the family, so a big drift still stands over a small one.
		assert!(family_peak("dune", 90) > family_peak("dune", 30));
		// ...and never flat: a body pixel is never bare ground.
		assert_eq!(family_peak("dune", 1), 1);
	}

	/// A piece the light can only find one flank of is not a step in the land,
	/// and half a wall is worth less than the dome it would replace.
	#[test]
	fn a_scarp_with_no_low_side_falls_back_to_the_dome() {
		let sprite = slab(21, 21, 100); // fills its frame: no ground either side
		let plain = HeightOpts { peak: 200, sunken: false, pyramid: false, scarp: false, rim: &[], foot: &[] };
		assert_eq!(
			sprite.height_field(&grey_palette(), &HeightOpts { scarp: true, ..plain }),
			sprite.height_field(&grey_palette(), &plain),
			"no flanks, no wall"
		);
	}

	/// **The outline stands at ground level, however the art lit it.** A slab
	/// painted its brightest ink along one edge used to read that edge as high
	/// ground - the luma said so and the dome was too weak there to argue.
	#[test]
	fn the_outline_meets_the_ground_however_bright_the_edge_is_painted() {
		let mut sprite = slab(21, 21, 40);
		for y in 0..21 {
			sprite.body[y * 21] = 255; // a sunlit western flank, right to the rim
		}
		let field = sprite.height_field(
			&grey_palette(),
			&HeightOpts { peak: 200, sunken: false, pyramid: false, scarp: false, rim: &[], foot: &[] },
		);
		let at = |x: usize, y: usize| field[y * 21 + x];
		assert_eq!(at(0, 10), 1, "the lit outline is ground level, not high ground");
		assert!(at(3, 10) > at(1, 10), "and it climbs out over the skirt: {} vs {}", at(3, 10), at(1, 10));
		assert!(at(10, 10) > at(3, 10), "the core still stands over all of it");
		// Every pixel of the silhouette, not just the one that was lit.
		let rim = (0..21).flat_map(|k| [at(k, 0), at(k, 20), at(0, k), at(20, k)]);
		assert!(rim.clone().all(|h| h == 1), "the whole outline: {:?}", rim.max());
	}

	/// A ribbon is edge all the way through, and sanding it flat would leave a
	/// SNOW_DARK cliff with no relief at all - so the skirt is only ever as wide
	/// as the piece can spare.
	#[test]
	fn a_thin_piece_keeps_the_relief_it_has() {
		let mut sprite = slab(40, 1, 40);
		for x in 20..40 {
			sprite.body[x] = 200;
		}
		let field = sprite.height_field(
			&grey_palette(),
			&HeightOpts { peak: 255, sunken: false, pyramid: false, scarp: false, rim: &[], foot: &[] },
		);
		assert!(field[35] > 1, "a one-pixel ribbon is not flattened: {}", field[35]);
	}

	/// The luma term, isolated: on a one-row sprite every pixel is equally deep
	/// inside the silhouette, so all that is left to separate them is how bright
	/// the art painted them - the signal CRATER and DESERT have and no other.
	#[test]
	fn a_lit_face_stands_over_a_dark_one() {
		let mut sprite = slab(40, 1, 40);
		for x in 20..40 {
			sprite.body[x] = 200;
		}
		let field = sprite.height_field(
			&grey_palette(),
			&HeightOpts { peak: 255, sunken: false, pyramid: false, scarp: false, rim: &[], foot: &[] },
		);
		assert!(field[35] > field[5], "the lit half is higher: {} vs {}", field[35], field[5]);
		// ...and it is still the luma when the object is a hole: dark is low
		// ground whichever way round the landform is.
		let sunk = sprite.height_field(
			&grey_palette(),
			&HeightOpts { peak: 255, sunken: true, pyramid: false, scarp: false, rim: &[], foot: &[] },
		);
		assert!(sunk[35] > sunk[5], "sunken flips the dome, not the light: {} vs {}", sunk[35], sunk[5]);
	}

	/// One flat tone says nothing about shape, and inventing relief out of the
	/// stretch's zero span would be a divide by zero besides.
	#[test]
	fn a_single_toned_object_leans_on_its_dome_alone() {
		let field = slab(9, 9, 77).height_field(
			&grey_palette(),
			&HeightOpts { peak: 100, sunken: false, pyramid: false, scarp: false, rim: &[], foot: &[] },
		);
		assert!(field[4 * 9 + 4] > field[0], "still domed: {} vs {}", field[4 * 9 + 4], field[0]);
	}

	/// The peak a piece falls back on, and the families that stand on their rims.
	#[test]
	fn the_inferred_relief_follows_the_sprite_and_the_family() {
		assert_eq!(slab(80, 40, 1).default_peak(), 20, "half the shorter side");
		assert_eq!(slab(1, 1, 1).default_peak(), 1, "never zero");
		assert_eq!(slab(2000, 900, 1).default_peak(), 255, "never past the byte");
		assert!(family_is_sunken("crater"), "the CRATER pack's craters are holes");
		assert!(!family_is_sunken("mountain") && !family_is_sunken("trees"));
	}

	/// **The renderer's ranking and the export's comparison are one rule.** The
	/// GPU cannot compare two inks per pixel, so it stores a brightness rank and
	/// lets `max` / `min` blending decide; that is only the same picture as
	/// [`SceneryBlend::pick`] if the rank order *is* the pick order.
	#[test]
	fn the_ink_ranking_agrees_with_pick() {
		// A palette with duplicate brightnesses, so the index tiebreak matters.
		let mut palette = vec![0u8; 768];
		for i in 0..256usize {
			let grey = (i / 2) as u8;
			set_slot_rgb(&mut palette, i as u8, [grey, grey, grey]);
		}
		let brightness = brightness_table(&palette);
		let (rank_of, ink_of) = ink_ranks(&palette);
		for (a, b) in [(1u8, 2), (2, 1), (7, 8), (255, 254), (40, 41), (99, 99)] {
			let brighter = SceneryBlend::Brighter.pick(a, b, 0, 0, &brightness);
			let darker = SceneryBlend::Darker.pick(a, b, 0, 0, &brightness);
			let (ra, rb) = (rank_of[a as usize], rank_of[b as usize]);
			assert_eq!(brighter, ink_of[ra.max(rb) as usize], "brighter({a},{b}) is the higher rank");
			assert_eq!(darker, ink_of[ra.min(rb) as usize], "darker({a},{b}) is the lower rank");
			assert_eq!(SceneryBlend::Normal.pick(a, b, 0, 0, &brightness), a, "normal is always the ink being painted");
		}
		// Every ink `1..=255` has exactly one rank, and none is the empty value.
		let mut seen = [false; 256];
		for ink in 1..=255u8 {
			let rank = rank_of[ink as usize];
			assert!(!seen[rank as usize], "rank {rank} used twice");
			seen[rank as usize] = true;
			assert_eq!(ink_of[rank as usize], ink, "the two tables are inverses");
			assert!(rank < 255, "a rank + 1 still fits a byte beside the empty 0");
		}
	}

	/// A piece hangs from its centre of *mass*, not its bounding box - and
	/// placing it puts that point exactly where asked, whatever the crop.
	#[test]
	fn a_piece_is_placed_by_its_centre_of_mass() {
		// A 10x10 box with body only in the left column: the box's middle is
		// x = 5, the mass is at x = 0.
		let mut body = vec![0u8; 100];
		for y in 0..10 {
			body[y * 10] = 9;
		}
		let sprite = Sprite { width: 10, height: 10, origin_x: 4, origin_y: 6, body, shade: vec![0; 100] };
		assert_eq!(sprite.center_of_mass(), (0, 4));
		let piece = SceneryPiece {
			id: "p".into(),
			name: "P".into(),
			family: "p".into(),
			transformable: Transformable::No,
			peak: None,
			sunken: None,
			scarp: None,
			height: None,
			cells_w: 1,
			cells_h: 1,
			pass: vec![PASS_EMPTY],
			sprite,
			user: true,
		};
		let (x, y) = piece.centered_at(100, 200);
		let spot = ScenerySpot { pack: "P".into(), piece: "p".into(), x, y, blend: SceneryBlend::default() };
		let (ox, oy) = piece.sprite_origin(&spot);
		let (cx, cy) = piece.sprite.center_of_mass();
		assert_eq!((ox + cx, oy + cy), (100, 200), "the mass lands on the point asked for");
	}

	/// The edge distance grows inward from every side and from a hole, and is
	/// zero wherever there is no body to be inside of.
	#[test]
	fn edge_distance_measures_how_deep_inside_the_object_a_pixel_is() {
		// A 9x9 block of ink with a one-pixel hole dead centre.
		let mut body = vec![7u8; 81];
		body[4 * 9 + 4] = 0;
		let sprite = Sprite { width: 9, height: 9, origin_x: 0, origin_y: 0, body, shade: vec![0; 81] };
		let d = sprite.edge_distance();
		let at = |x: usize, y: usize| d[y * 9 + x];
		assert_eq!(at(4, 4), 0, "the hole itself is not inside anything");
		assert_eq!(at(0, 0), 1, "a corner is one step in");
		assert_eq!(at(4, 0), 1, "so is an edge pixel");
		assert_eq!(at(4, 3), 1, "and one beside the hole");
		assert_eq!(at(2, 2), 3, "three rows in from two sides is three");
		assert!(at(0, 4) < at(2, 4), "the distance grows inward");
	}

	/// The dither is a ramp: the rim nearly always gives way, `BLEND_BAND` deep
	/// always wins, and in between the share rises with depth.
	#[test]
	fn blend_keeps_ramps_over_the_band() {
		let share = |edge: u8| {
			(0..8).flat_map(|y| (0..8).map(move |x| (x, y))).filter(|&(x, y)| blend_keeps(edge, x, y)).count()
		};
		assert_eq!(share(0), 0, "the rim always gives way");
		assert_eq!(share(BLEND_BAND), 64, "a full band deep always wins");
		assert_eq!(share(255), 64, "and so does anything deeper");
		let (near, mid) = (share(2), share(10));
		assert!(0 < near && near < mid && mid < 64, "the share rises with depth: {near} < {mid}");
		// Half the band through, about half the pixels survive.
		assert!((share(BLEND_BAND / 2) as i32 - 32).abs() <= 4, "the ramp is even");
	}

	/// A hand-authored image overrules everything the flood would have guessed:
	/// its alpha says what is nothing, what shades and what is the object, and
	/// ground-toned pixels it keeps are the object's own lit faces.
	#[test]
	fn a_hand_authored_image_decides_what_the_flood_cannot() {
		// 4x3: a row of ink over a row of shadow over a row of nothing, plus a
		// ground-toned pixel the artist kept and a barely-opaque one.
		let (w, h) = (4usize, 3usize);
		let mut rgba = vec![0u8; w * h * 4];
		let put = |rgba: &mut [u8], x: usize, y: usize, px: [u8; 4]| {
			rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&px);
		};
		let mut palette = vec![0u8; 768];
		set_slot_rgb(&mut palette, 60, [200, 20, 20]); // the object's ink
		set_slot_rgb(&mut palette, 61, [40, 160, 40]); // the ground tone
		for x in 0..w {
			put(&mut rgba, x, 0, [200, 20, 20, 255]);
			put(&mut rgba, x, 1, [0, 0, 0, 128]); // dead centre of the band
		}
		put(&mut rgba, 0, 0, [40, 160, 40, 255]); // kept ground: a lit face
		put(&mut rgba, 3, 0, [200, 20, 20, 20]); // barely opaque, still ink
		// Row 2 is left fully transparent, so the crop drops it.

		let sprite = cut_image(&rgba, w, h, &palette);
		assert_eq!((sprite.width, sprite.height), (4, 2), "the empty row is cropped away");
		let at = |x: usize, y: usize| y * sprite.width as usize + x;
		assert_eq!(sprite.body[at(1, 0)], 60, "opaque paint is the object's ink");
		assert_eq!(sprite.body[at(0, 0)], 61, "a ground tone the artist kept is a lit face, not ground");
		assert_eq!(sprite.body[at(3, 0)], 60, "and 8% alpha is paint too - only 0 is nothing");
		assert_eq!(sprite.shade[at(1, 1)], SHADOW_ALPHA, "the half-alpha row shades");
		assert_eq!(sprite.body[at(1, 1)], 0, "a shadow pixel carries no ink");
		assert!(cut_image(&[0; 4 * 4], 2, 2, &palette).is_empty(), "an all-transparent image is no object");
	}

	#[test]
	fn close_seals_a_dithered_channel_into_the_interior() {
		let (mut src, ground, shadow) = fixture();
		// Punch a one-pixel channel through the block's left wall, so the flood
		// can reach the interior pixel the way a dithered edge lets it.
		src[3 * 8 + 2] = Some(10);
		let open = cut(&src, 8, 8, &ground, &shadow, &CutOpts { close: 0, alpha: 100 });
		let at = |s: &Sprite, x: usize, y: usize| s.body[y * s.width as usize + x];
		assert_eq!(at(&open, 2, 1), 0, "without a close the flood reaches in");
		let sealed = cut(&src, 8, 8, &ground, &shadow, &CutOpts { close: 1, alpha: 100 });
		assert_eq!(at(&sealed, 2, 1), 10, "close 1 seals the 1px channel");
	}

	#[test]
	fn an_all_ground_box_cuts_to_nothing() {
		let ground = GroundInk {
			is_ground: {
				let mut g = [false; 256];
				g[10] = true;
				g
			},
		};
		let src = vec![Some(10u8); 64];
		let sprite = cut(&src, 8, 8, &ground, &ShadowInk::new(), &CutOpts::default());
		assert!(sprite.is_empty(), "nothing but ground leaves no object");
	}

	#[test]
	fn holes_are_not_body() {
		let ground = GroundInk { is_ground: [false; 256] };
		let mut src = vec![None; 16];
		src[5] = Some(7);
		let sprite = cut(&src, 4, 4, &ground, &ShadowInk::new(), &CutOpts::default());
		assert_eq!((sprite.width, sprite.height, sprite.origin_x, sprite.origin_y), (1, 1, 1, 1));
		assert_eq!(sprite.body, vec![7]);
	}

	#[test]
	fn planes_round_trip_through_the_run_encoding() {
		let mut plane = vec![0u8; 7 * 5];
		plane[0] = 1; // run at the row start
		plane[6] = 2; // run at the row end
		plane[7 + 3] = 3; // run in the middle
		for x in 0..7 {
			plane[2 * 7 + x] = 9; // a full row
		}
		let data = encode_plane(&plane, 7, 5);
		assert_eq!(decode_plane(&data, 7, 5).expect("decodes"), plane);
		assert!(data.len() < plane.len() * 2, "sparse rows encode small");
	}

	#[test]
	fn a_truncated_plane_is_an_error_not_a_panic() {
		let data = encode_plane(&[1u8; 16], 4, 4);
		assert!(decode_plane(&data[..data.len() - 1], 4, 4).is_err(), "short data errors");
		assert!(decode_plane(&[], 4, 4).is_err(), "empty data errors");
	}

	#[test]
	fn a_pack_round_trips_through_the_asset_format() {
		let (src, ground, shadow) = fixture();
		let sprite = cut(&src, 8, 8, &ground, &shadow, &CutOpts { close: 0, alpha: 100 });
		let pack = SceneryPack {
			pack: "GREEN".to_string(),
			pieces: vec![SceneryPiece {
				id: "mountain-3".to_string(),
				name: "Mountain 3".to_string(),
				family: "mountain".to_string(),
				transformable: Transformable::Invert,
				peak: None,
				sunken: None,
				scarp: None,
				height: None,
				cells_w: 2,
				cells_h: 1,
				pass: vec![3, PASS_EMPTY],
				sprite: sprite.clone(),
				user: false,
			}],
		};
		let back = round_trip(&pack, "one-piece");
		assert_eq!(back.pieces.len(), 1);
		assert_eq!(back.pieces[0].id, "mountain-3");
		assert_eq!(back.pieces[0].name, "Mountain 3");
		assert_eq!(back.pieces[0].pass, vec![3, PASS_EMPTY]);
		assert_eq!(back.pieces[0].sprite, sprite);
		assert_eq!(back.pieces[0].family, "mountain", "the authored family survives");
		assert_eq!(back.pieces[0].transformable, Transformable::Invert, "and so does the transform");
	}

	/// A piece whose files say nothing about family or transform still loads,
	/// with each prefilled the way the bake would have written it.
	#[test]
	fn a_piece_without_family_or_transform_falls_back() {
		let dir = scratch("bare-meta");
		let pack = SceneryPack { pack: "GREEN".to_string(), pieces: vec![a_piece("mountain-3")] };
		pack.save(&dir).expect("saves");
		// The `.json` is an overlay, so stripping it back to a name is a thing a
		// user can do - and the piece is still a piece.
		let meta = dir.join(SCENERY_DIR).join("GREEN").join("mountain-3.json");
		std::fs::write(&meta, r#"{"name": "Mountain 3"}"#).expect("writes");
		let back = SceneryPack::load(&dir, "GREEN").expect("loads");
		assert_eq!(back.pieces[0].name, "Mountain 3", "the overlay's one field took");
		assert_eq!(back.pieces[0].family, "piece", "and the rest came off the .scn");
		assert_eq!(back.pieces[0].transformable, Transformable::No);
		assert_eq!(back.pieces[0].sprite, pack.pieces[0].sprite);
	}

	/// An authored relief round-trips through the library's files; an inferred
	/// one writes no keys and no `.hgt` at all, so a library that leaves every
	/// piece on `auto` carries nothing about relief.
	#[test]
	fn an_authored_relief_round_trips_and_an_inferred_one_writes_nothing() {
		let dir = scratch("relief");
		let plain = SceneryPack { pack: "GREEN".to_string(), pieces: vec![a_piece("mountain-3")] };
		plain.save(&dir).expect("saves");
		let green = dir.join(SCENERY_DIR).join("GREEN");
		let text = std::fs::read_to_string(green.join("mountain-3.json")).expect("a meta file");
		assert!(!text.contains("peak") && !text.contains("sunken"), "nothing written for auto: {text}");
		assert!(!green.join(format!("mountain-3.{HGT_EXT}")).exists(), "and no height map");

		let authored = SceneryPiece { peak: Some(200), sunken: Some(true), ..a_piece("crater-3") };
		let back = round_trip(&SceneryPack { pack: "GREEN".to_string(), pieces: vec![authored] }, "relief-authored");
		assert_eq!((back.pieces[0].peak, back.pieces[0].sunken), (Some(200), Some(true)));
		let opts = back.pieces[0].height_opts();
		assert!(opts.sunken && opts.peak == 200, "and it is what the relief is measured with");
		// The override really overrides: the family says nothing about it either
		// way, and the inferred peak is not 200.
		assert!(!family_is_sunken("piece"));
		assert_ne!(back.pieces[0].sprite.default_peak(), 200);
	}

	/// **The library is a directory of pieces.** A piece is three files under
	/// its id, the two overlays win over the `.scn` they sit beside, and a save
	/// takes the files of a piece that is gone with it.
	#[test]
	fn a_library_is_a_directory_of_pieces() {
		let dir = scratch("directory");
		let mut lib = SceneryPack {
			pack: "GREEN".to_string(),
			pieces: vec![a_piece("mountain-3"), a_piece("crater-1"), a_piece("trees-2")],
		};
		let texels = lib.pieces[1].sprite.width as usize * lib.pieces[1].sprite.height as usize;
		lib.pieces[1].height = Some(vec![17u8; texels]);
		lib.save(&dir).expect("saves");

		let green = dir.join(SCENERY_DIR).join("GREEN");
		for id in ["mountain-3", "crater-1", "trees-2"] {
			assert!(green.join(format!("{id}.{SCN_EXT}")).is_file(), "{id} has a piece file");
			assert!(green.join(format!("{id}.json")).is_file(), "{id} has a meta file");
		}
		assert!(green.join(format!("crater-1.{HGT_EXT}")).is_file(), "the drawn relief is its own file");
		assert!(!green.join(format!("trees-2.{HGT_EXT}")).exists(), "and an inferred one is no file");

		// The pieces come back in id order, whatever order they were written in.
		let back = SceneryPack::load(&dir, "GREEN").expect("loads");
		assert_eq!(
			back.pieces.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
			["crater-1", "mountain-3", "trees-2"]
		);
		assert_eq!(back.pieces[0].height, Some(vec![17u8; texels]), "the .hgt came back with it");
		assert!(back.pieces.iter().all(|p| !p.user), "a library's pieces belong to the root they came from");

		// A hand-edited overlay wins over the `.scn` it sits beside.
		std::fs::write(
			green.join("trees-2.json"),
			r#"{"name": "Renamed", "family": "grove", "transform": "invert", "peak": 77}"#,
		)
		.expect("writes");
		let back = SceneryPack::load(&dir, "GREEN").expect("loads");
		let edited = back.piece("trees-2").expect("still there");
		assert_eq!((edited.name.as_str(), edited.family.as_str()), ("Renamed", "grove"));
		assert_eq!((edited.transformable, edited.peak), (Transformable::Invert, Some(77)));

		// A height map drawn for a frame the art no longer has is dropped, not
		// fatal: the piece falls back to whatever it would have had without the
		// file - here the inference, since `trees-2` was never drawn one.
		std::fs::write(green.join(format!("trees-2.{HGT_EXT}")), write_hgt(&[1, 2, 3, 4], 2, 2)).expect("writes");
		let back = SceneryPack::load(&dir, "GREEN").expect("a stale height map is not an error");
		assert!(!back.piece("trees-2").expect("still there").height_authored(), "the stale plane was dropped");

		// ...and a save is what deletes a piece, files and all.
		let mut lib = SceneryPack::load(&dir, "GREEN").expect("loads");
		lib.pieces.retain(|p| p.id != "mountain-3");
		lib.save(&dir).expect("saves");
		for ext in [SCN_EXT, "json", HGT_EXT] {
			assert!(!green.join(format!("mountain-3.{ext}")).exists(), "the dropped piece's .{ext} went with it");
		}
		assert_eq!(SceneryPack::load(&dir, "GREEN").expect("loads").pieces.len(), 2);
	}

	/// **A relief survives the trip out to a picture and back**, which is the
	/// whole of the editing loop: look at it, paint on it, bring it back. The
	/// image is stretched to its own white, so it is the peak that puts the
	/// numbers back where they were.
	#[test]
	fn a_relief_round_trips_through_a_painted_picture() {
		let piece = a_piece("mountain-3");
		let (sw, sh) = (piece.sprite.width, piece.sprite.height);
		let peak = 90u8;
		let field: Vec<u8> = (0..sw as usize * sh as usize)
			.map(|i| if piece.sprite.body[i] == 0 { 0 } else { (i % peak as usize + 1) as u8 })
			.collect();

		let grey = height_to_grey(&field, peak);
		assert_eq!(height_to_grey(&[peak], peak), vec![255], "white is the peak, whatever the art reaches");
		let back = height_from_grey(&grey, sw as usize, sh as usize, &piece.sprite, piece.cells_w, piece.cells_h, peak)
			.expect("the sprite's own frame is a frame it knows");
		// Through a byte and back, so within the rounding the stretch costs.
		for (i, (&was, &now)) in field.iter().zip(&back).enumerate() {
			assert!(was.abs_diff(now) <= 1, "pixel {i}: {was} -> {now}");
		}

		// The body plane, not the painter, says where the object is: paint
		// outside it and the paint is dropped; leave a body pixel black and it
		// still stands at 1, because 0 is the height of bare ground.
		let flat = height_from_grey(
			&vec![0u8; sw as usize * sh as usize],
			sw as usize,
			sh as usize,
			&piece.sprite,
			piece.cells_w,
			piece.cells_h,
			peak,
		)
		.expect("a frame it knows");
		for i in 0..flat.len() {
			assert_eq!(flat[i], u8::from(piece.sprite.body[i] != 0), "pixel {i} follows the silhouette");
		}

		// A picture in the footprint's frame is the other one it knows...
		let (bw, bh) = (piece.cells_w as usize * 64, piece.cells_h as usize * 64);
		assert!(
			height_from_grey(&vec![255u8; bw * bh], bw, bh, &piece.sprite, piece.cells_w, piece.cells_h, peak)
				.is_some()
		);
		// ...and anything else is not a height map for this piece.
		assert!(height_from_grey(&[255u8; 9], 3, 3, &piece.sprite, piece.cells_w, piece.cells_h, peak).is_none());
	}

	/// A scratch resources root of this test's own.
	fn scratch(tag: &str) -> std::path::PathBuf {
		let dir = std::env::temp_dir().join(format!("mme-scenery-{}-{tag}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		dir
	}

	/// `pack` written to a scratch root and read straight back.
	fn round_trip(pack: &SceneryPack, tag: &str) -> SceneryPack {
		let dir = scratch(tag);
		pack.save(&dir).expect("saves");
		SceneryPack::load(&dir, &pack.pack).expect("loads")
	}

	/// **An authored height map replaces the inference outright**, and a piece
	/// without one still gets the guess - which is the whole of the fallback the
	/// renderer, the bake and the ghost rely on.
	#[test]
	fn an_authored_height_map_wins_and_a_missing_one_falls_back() {
		let grey = grey_palette();
		let inferred = a_piece("mountain-3");
		let guess = inferred.height_field(&grey);
		assert!(!inferred.height_authored(), "nothing authored");
		assert_eq!(guess, inferred.sprite.height_field(&grey, &inferred.height_opts()), "so it is the inference");

		let texels = inferred.sprite.width as usize * inferred.sprite.height as usize;
		let drawn = (0..texels).map(|i| (i % 250 + 1) as u8).collect::<Vec<u8>>();
		let authored = SceneryPiece { height: Some(drawn.clone()), ..inferred.clone() };
		assert!(authored.height_authored());
		assert_eq!(authored.height_field(&grey), drawn, "the drawn relief, verbatim");
		// The peak and sunken overrides are inputs to the *inference* - an authored
		// plane is the answer itself, so they no longer have anything to say.
		let with_opts = SceneryPiece { peak: Some(9), sunken: Some(true), ..authored.clone() };
		assert_eq!(with_opts.height_field(&grey), drawn, "and nothing re-scales it");

		// A plane that does not fit its sprite can only be a stale file beside a
		// re-cut piece: worth less than the guess, so it is treated as absent.
		let stale = SceneryPiece { height: Some(vec![7u8; texels + 1]), ..inferred.clone() };
		assert!(!stale.height_authored());
		assert_eq!(stale.height_field(&grey), guess, "a mis-sized relief falls back to the inference");
	}

	/// The `.hgt` container: a relief round-trips through it, and every way of
	/// being not-a-height-map is an error rather than a panic.
	#[test]
	fn a_relief_round_trips_through_the_hgt_container() {
		let (w, h) = (7u16, 5u16);
		let plane: Vec<u8> = (0..(w as usize * h as usize)).map(|i| if i % 3 == 0 { 0 } else { i as u8 }).collect();
		let bytes = write_hgt(&plane, w, h);
		assert_eq!(read_hgt(&bytes).expect("round-trips"), (plane, w, h));

		assert!(read_hgt(&[]).is_err(), "empty");
		assert!(read_hgt(b"not a height map at all").is_err(), "bad magic");
		assert!(read_hgt(&bytes[..HGT_MAGIC.len() + 2]).is_err(), "truncated header");
		assert!(read_hgt(&bytes[..bytes.len() - 3]).is_err(), "truncated plane");
	}

	/// A shared `.scn` carries the relief with the art, so a piece handed to
	/// someone else stands as high for them as it does here.
	#[test]
	fn a_bundled_scn_carries_an_authored_relief() {
		let piece = a_piece("mountain-3");
		let texels = piece.sprite.width as usize * piece.sprite.height as usize;
		let drawn = vec![42u8; texels];
		let (back, _) = read_scn(&write_scn(&SceneryPiece { height: Some(drawn.clone()), ..piece.clone() }, "GREEN"))
			.expect("round-trips");
		assert_eq!(back.height, Some(drawn));
		// ...and a piece that infers its relief writes no third plane at all.
		let (plain, _) = read_scn(&write_scn(&piece, "GREEN")).expect("round-trips");
		assert_eq!(plain.height, None);
	}

	/// **CRATER ships its relief as data.** Every piece in it carries a `.hgt`
	/// sized to its own art, so what the renderer stands the object at is a file
	/// somebody can open - not a number re-derived per session. The other four
	/// packs infer, which is still a supported way for a piece to live, and this
	/// says which is which rather than assuming.
	#[test]
	fn the_crater_pack_ships_a_height_map_for_every_piece() {
		let pack = SceneryPack::load(&assets_root(), "CRATER").expect("CRATER loads");
		let inferred: Vec<&str> = pack.pieces.iter().filter(|p| !p.height_authored()).map(|p| p.id.as_str()).collect();
		assert!(inferred.is_empty(), "CRATER pieces with no height map: {inferred:?}");

		// All eleven craters stand on the ground at their outline and are still an
		// object in the middle - the two ends of the profile, checked everywhere.
		let craters: Vec<&SceneryPiece> = pack.pieces.iter().filter(|p| p.id.starts_with("crater-")).collect();
		assert_eq!(craters.len(), 11, "the pack's craters");
		for crater in &craters {
			let field = crater.height_field(&[0; 256]);
			let (id, w) = (&crater.id, crater.sprite.width as usize);
			let row = crater.sprite.height as usize / 2;
			let at = |x: usize| field[row * w + x] as usize;
			assert!(at(0) <= 1, "{id}: the outline stands at ground level, not {}", at(0));
			assert!(at(w / 2) > 0, "{id}: and the middle is still the object, not a hole in it");
		}

		// ...and the big one really does read as a crater: up to a raised rim well
		// outside the middle, then down into the bowl. Measured on one piece
		// rather than all eleven because it is a claim about a *shape*, and the
		// small craters' middle rows are a few dozen pixels of nearly flat floor
		// where the tallest of them is noise. This is the one shape a plain dome
		// gets exactly upside down, and `crater-1` is where it shows.
		let crater = pack.piece("crater-1").expect("crater-1 is in the pack");
		let field = crater.height_field(&[0; 256]);
		let w = crater.sprite.width as usize;
		let row = crater.sprite.height as usize / 2;
		let at = |x: usize| field[row * w + x] as usize;
		let rim = (0..w / 2).max_by_key(|&x| at(x)).expect("a tallest pixel on the row");
		assert!(rim < w / 3, "the rim stands well outside the middle, not at {rim} of {w}");
		// A margin of a quarter, not the half this asserted before the rim was
		// traced: the tracing put the crest where the art has it rather than at 35%
		// of the depth, and on this piece that is further out, over ground the luma
		// reads as darker.
		assert!(at(rim) * 4 > at(w / 2) * 5, "and over the bowl: {} vs {}", at(rim), at(w / 2));
	}

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets")
	}

	/// **CRATER casts nothing, and that is the art rather than a broken bake.**
	/// Its 46 pieces are all hand-cut, and the alpha rule is the only shadow a
	/// hand cut has: no source PNG paints a half-alpha pixel, so there is no
	/// shadow to bake. `bake_scenery` says so about every one of them.
	///
	/// A named exemption rather than a softened assertion, and asserted in both
	/// directions below - the day the art gains a drawn shadow, this list is what
	/// says so out loud.
	const SHADOWLESS: [&str; 1] = ["CRATER"];

	/// The shipped bake loads, and every piece holds to the format's promises:
	/// planes sized to the sprite, body and shade never both set, a pass grid
	/// that matches the footprint, and at least one shaded pixel somewhere in
	/// the pack (or the shadow tuning silently did nothing) unless the pack is
	/// [`SHADOWLESS`].
	#[test]
	fn the_shipped_scenery_loads_and_is_well_formed() {
		for name in ["CRATER", "DESERT", "GREEN", "SNOW", "SNOW_DARK"] {
			let pack = SceneryPack::load(&assets_root(), name).unwrap_or_else(|e| panic!("{name}: {e}"));
			assert!(!pack.pieces.is_empty(), "{name} has objects");
			let mut shaded = 0usize;
			for piece in &pack.pieces {
				let n = piece.sprite.width as usize * piece.sprite.height as usize;
				assert!(n > 0, "{name}/{}: empty sprite was baked", piece.id);
				assert_eq!(piece.sprite.body.len(), n, "{name}/{}: body plane", piece.id);
				assert_eq!(piece.sprite.shade.len(), n, "{name}/{}: shade plane", piece.id);
				assert_eq!(
					piece.pass.len(),
					piece.cells_w as usize * piece.cells_h as usize,
					"{name}/{}: pass grid",
					piece.id
				);
				for i in 0..n {
					assert!(
						piece.sprite.body[i] == 0 || piece.sprite.shade[i] == 0,
						"{name}/{}: pixel {i} is both ink and shadow",
						piece.id
					);
				}
				// The sprite is cropped, so it cannot exceed its footprint.
				assert!(
					piece.sprite.origin_x as usize + piece.sprite.width as usize <= piece.cells_w as usize * 64,
					"{name}/{}: sprite is wider than its cells",
					piece.id
				);
				shaded += piece.sprite.shade.iter().filter(|&&s| s != 0).count();
			}
			match SHADOWLESS.contains(&name) {
				false => assert!(shaded > 0, "{name}: no pixel shades - check tune.json's shadow set"),
				true => assert_eq!(
					shaded, 0,
					"{name} is listed SHADOWLESS but now shades {shaded} pixel(s) - the art gained a half-alpha \
					 shadow, so take it off the list"
				),
			}
		}
	}

	/// The alpha the shipped GREEN bake carries, run through [`ShadeTable`],
	/// lands the pack's commonest grass inks on **238** - the very ink the
	/// original art shades grass with. This is the join between the tuning file,
	/// the palette and the export: change the alpha, the colour space, or the
	/// nearest-slot rule and a GREEN shadow stops matching the art.
	#[test]
	fn the_shipped_green_shadow_lands_on_the_arts_own_ink() {
		let tiles = assets_root().join("tilepacks");
		let pack = TilePack::load(&tiles, "GREEN").expect("GREEN loads");
		let mut palette = pack.palette.clone().expect("GREEN owns a palette");
		crate::game_palette::apply_game_statics(&mut palette);
		let scenery = SceneryPack::load(&assets_root(), "GREEN").expect("GREEN scenery loads");
		let alpha = scenery
			.pieces
			.iter()
			.flat_map(|piece| piece.sprite.shade.iter())
			.copied()
			.find(|&a| a != 0)
			.expect("the GREEN bake shades something");
		let table = ShadeTable::build(&palette, alpha);
		for grass in [79u8, 72, 74, 82, 77, 76] {
			assert_eq!(table.apply(grass), 238, "grass ink {grass} shades to the art's own shadow ink");
		}
		// And a shadow never lands somewhere the engine colour-cycles, which
		// would shimmer in-game.
		for index in 0..=255u8 {
			let shaded = table.apply(index);
			assert!(shaded != 0 && !crate::deanimate::animated_slot(shaded), "index {index} shaded onto {shaded}");
		}
	}

	/// A piece file from a format this build does not speak is refused by name,
	/// rather than half-read into something that looks like a cut-out.
	#[test]
	fn a_wrong_version_is_rejected() {
		let mut bytes = write_scn(&a_piece("mountain-3"), "GREEN");
		// Patch the version inside the manifest in place - same length, so the
		// header's manifest length and the blob behind it both stay valid.
		let at = SCN_MAGIC.len() + 4;
		let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
		let text =
			std::str::from_utf8(&bytes[at..at + len]).expect("utf-8").replace(r#""version": "1""#, r#""version": "9""#);
		assert_eq!(text.len(), len, "the patch is the same size");
		bytes[at..at + len].copy_from_slice(text.as_bytes());
		let err = read_scn(&bytes).unwrap_err();
		assert!(err.contains("version"), "{err}");
	}

	fn a_piece(id: &str) -> SceneryPiece {
		let (src, ground, shadow) = fixture();
		SceneryPiece {
			id: id.to_string(),
			family: "piece".to_string(),
			transformable: Transformable::No,
			peak: None,
			sunken: None,
			scarp: None,
			height: None,
			name: format!("Piece {id}"),
			cells_w: 2,
			cells_h: 1,
			pass: vec![3, PASS_EMPTY],
			sprite: cut(&src, 8, 8, &ground, &shadow, &CutOpts { close: 0, alpha: 100 }),
			user: false,
		}
	}

	/// A shared `.scn` carries one whole piece plus the palette it was drawn
	/// against, and comes back marked as the user's.
	#[test]
	fn a_piece_round_trips_through_the_scn_container() {
		let piece = a_piece("oak-stand");
		let bytes = write_scn(&piece, "GREEN");
		assert!(bytes.starts_with(SCN_MAGIC), "the magic leads");
		let (back, pack) = read_scn(&bytes).expect("round-trips");
		assert_eq!(pack, "GREEN", "the source pack rides along as a hint");
		assert_eq!((back.id, back.name), (piece.id, piece.name));
		assert_eq!((back.cells_w, back.cells_h), (2, 1));
		assert_eq!(back.pass, vec![3, PASS_EMPTY]);
		assert_eq!(back.sprite, piece.sprite);
		assert!(back.user, "an imported piece is the user's");

		// Every way of being not-a-scn is an error, never a panic.
		assert!(read_scn(&[]).is_err(), "empty");
		assert!(read_scn(b"not a scenery file at all").is_err(), "bad magic");
		assert!(read_scn(&bytes[..SCN_MAGIC.len() + 2]).is_err(), "truncated header");
		assert!(read_scn(&bytes[..SCN_MAGIC.len() + 8]).is_err(), "truncated manifest");
		let mut short = bytes.clone();
		short.truncate(short.len() - 1);
		assert!(read_scn(&short).is_err(), "truncated blob");
	}

	/// The user root layers over the shipped one: fresh ids join the library,
	/// a colliding id replaces the shipped piece, and only the user's are
	/// flagged - which is what makes rename and delete safe.
	#[test]
	fn the_user_root_layers_over_the_shipped_library() {
		let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/scenery-merge-test");
		let _ = std::fs::remove_dir_all(&dir);
		let (assets, user) = (dir.join("assets"), dir.join("user"));
		SceneryPack { pack: "GREEN".into(), pieces: vec![a_piece("shipped-1"), a_piece("shared")] }
			.save(&assets)
			.expect("shipped library writes");
		let mine = SceneryPiece { name: "Mine".into(), ..a_piece("shared") };
		SceneryPack { pack: "GREEN".into(), pieces: vec![a_piece("mine-1"), mine] }
			.save(&user)
			.expect("user library writes");

		let merged = SceneryPack::load_merged(&assets, &user, "GREEN").expect("both roots hold GREEN");
		let by_id = |id: &str| merged.pieces.iter().find(|p| p.id == id).unwrap_or_else(|| panic!("{id} is listed"));
		assert_eq!(merged.pieces.len(), 3, "two shipped + one fresh user piece");
		assert!(!by_id("shipped-1").user, "a shipped piece is not the user's to delete");
		assert!(by_id("mine-1").user);
		assert_eq!(by_id("shared").name, "Mine", "a colliding user id replaces the shipped piece");
		assert!(by_id("shared").user, "and takes its ownership with it");

		// Only the user's half is ever written back.
		let subset = merged.user_subset();
		// In library order: the replacement kept the shipped piece's slot, the
		// fresh one appended.
		assert_eq!(subset.pieces.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["shared", "mine-1"]);

		// Either root alone is enough; neither means the pack simply isn't listed.
		assert!(SceneryPack::load_merged(&assets, &dir.join("nothing"), "GREEN").is_some());
		assert!(SceneryPack::load_merged(&dir.join("nothing"), &user, "GREEN").is_some());
		assert!(SceneryPack::load_merged(&assets, &user, "SNOW").is_none(), "a pack in neither root is absent");
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// The three alpha bands land in the two planes, the sprite crops to what
	/// survived while the footprint stays the source image's, and a cell blocks
	/// only once the body really covers it.
	#[test]
	fn an_image_rasterizes_into_the_two_planes() {
		// A 128x64 image (2x1 cells): the left cell solidly red, the right cell
		// holding a small block of half-alpha shadow and a scrap of ink too
		// small to block.
		let (w, h) = (128usize, 64usize);
		let mut rgba = vec![0u8; w * h * 4];
		let put = |rgba: &mut [u8], x: usize, y: usize, px: [u8; 4]| {
			rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&px);
		};
		for y in 8..56 {
			for x in 8..56 {
				put(&mut rgba, x, y, [200, 20, 20, 255]); // opaque ink
			}
		}
		for y in 20..30 {
			for x in 70..80 {
				put(&mut rgba, x, y, [0, 0, 0, 160]); // the shadow band
			}
		}
		put(&mut rgba, 100, 40, [10, 200, 10, 255]); // one ink pixel: too little to block
		put(&mut rgba, 64, 40, [10, 200, 10, 50]); // barely there, but painted: still ink
		put(&mut rgba, 90, 12, [10, 200, 10, 0]); // fully transparent: nothing at all

		let mut palette = vec![0u8; 768];
		set_slot_rgb(&mut palette, 60, [210, 30, 30]);
		set_slot_rgb(&mut palette, 61, [20, 210, 20]);
		let opts = RasterOpts::default();
		let (sprite, pass, cells_w, cells_h) = rasterize(&rgba, w, h, &palette, &opts).expect("something survived");

		assert_eq!((cells_w, cells_h), (2, 1), "the footprint is the source image, in cells");
		assert_eq!(pass, vec![3, PASS_EMPTY], "only the solid cell blocks");
		// Cropped to x 8..=100, y 8..=55; the origin is where that sits in the image.
		assert_eq!((sprite.origin_x, sprite.origin_y), (8, 8));
		assert_eq!((sprite.width, sprite.height), (93, 48));
		let at = |x: usize, y: usize| (y - 8) * sprite.width as usize + (x - 8);
		assert_eq!(sprite.body[at(20, 20)], 60, "opaque red quantized onto the nearest usable slot");
		assert_eq!(sprite.shade[at(20, 20)], 0, "and carries no shadow");
		assert_eq!(sprite.shade[at(72, 22)], SHADOW_ALPHA, "the half-alpha band is shadow");
		assert_eq!(sprite.body[at(72, 22)], 0, "and no ink");
		assert_eq!(sprite.body[at(100, 40)], 61, "the lone ink pixel is kept, and set the crop's right edge");
		assert_eq!(sprite.body[at(64, 40)], 61, "a barely-opaque pixel is ink the artist painted, not nothing");
		assert_eq!(sprite.shade[at(64, 40)], 0, "and not shadow either - only the half band shades");
		assert_eq!((sprite.body[at(90, 12)], sprite.shade[at(90, 12)]), (0, 0), "only *fully* transparent is nothing");

		// A wholly transparent image is no piece at all.
		let blank = vec![0u8; w * h * 4];
		assert!(rasterize(&blank, w, h, &palette, &opts).is_none());
		assert!(rasterize(&rgba, 0, 0, &palette, &opts).is_none(), "an empty image is not a piece");
		assert!(rasterize(&rgba[..8], w, h, &palette, &opts).is_none(), "a short buffer is refused, not indexed");
	}

	/// Body ink never lands on slot 0 (which means "nothing here") nor on a slot
	/// the engine colour-cycles - an imported object must not shimmer.
	#[test]
	fn body_ink_avoids_slot_zero_and_the_cycled_bands() {
		let mut palette = vec![0u8; 768];
		// Make the *exact* match a cycled slot and slot 0, so a naive nearest
		// search would pick one of them.
		set_slot_rgb(&mut palette, 0, [255, 0, 0]);
		set_slot_rgb(&mut palette, 20, [255, 0, 0]); // ANIMATED_SLOTS
		set_slot_rgb(&mut palette, 100, [0, 0, 255]); // WATER_SLOTS
		set_slot_rgb(&mut palette, 200, [250, 10, 10]);
		set_slot_rgb(&mut palette, 201, [10, 10, 250]);
		assert_eq!(nearest_body_index(&palette, [255, 0, 0]), 200);
		assert_eq!(nearest_body_index(&palette, [0, 0, 255]), 201);
	}
}
