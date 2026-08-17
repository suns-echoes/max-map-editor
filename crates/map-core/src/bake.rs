//! WRL export bake: flatten a project's cell stacks into a
//! flat WRL - `compose_cell` per cell, byte-identical tiles deduplicated,
//! pass from the stack-top tile's pack data, minimap from the composed
//! center pixel (the original game's own derivation is unknown - see
//! `docs/design/tileset-contract.md` §4).
//!
//! Scenery (`SCENERY.md`) is painted over each composed cell here rather than
//! in `compose_cell`, which stays a pure flattening of the cell's *stack*: a
//! placement is positioned by pixel, so it belongs to the map, not to any one
//! cell's tiles. A map with no scenery never enters that path and bakes exactly
//! as it did before scenery existed.

use std::collections::HashMap;

use max_assets::wrl::{TILE_DATA_SIZE, TILE_SIZE, WrlFile};

use crate::project::{LAYER_GROUND, LAYER_WATER, Project, TileRef, Transform};
use crate::scenery::{SceneryBlend, ShadeTable};

/// `"WRL" 1 0` - the header all retail maps carry (demo maps use `DMO`).
pub const WRL_HEADER: [u8; 5] = [b'W', b'R', b'L', 1, 0];

/// Tile budget: bigmap indices and `tile_count` are u16.
pub const MAX_BAKED_TILES: usize = 65_535;

const CENTER_PIXEL: usize = 32 * 64 + 32;

pub fn bake(project: &Project) -> Result<WrlFile, String> {
	let (width, height) = (project.width, project.height);
	let cell_count = width as usize * height as usize;

	let mut tiles: Vec<u8> = Vec::new();
	let mut pass_table: Vec<u8> = Vec::new();
	let mut index_of: HashMap<[u8; TILE_DATA_SIZE], u16> = HashMap::new();
	let mut bigmap = Vec::with_capacity(cell_count);
	let mut minimap = Vec::with_capacity(cell_count);

	// Water-phase canonicalization: under a ground tile's cut-outs, any sea
	// phase is visually equivalent (animated noise) - the originals bake
	// exactly one phase per shore art. Re-pointing the water layer at the
	// water pack's tile 0 before composing keeps the dedup as tight as
	// Interplay's. Open-water cells keep their per-cell pattern tiles.
	let canonical_water = project.water_pack.map(|pack| TileRef { pack, tile: 0, transform: Transform::default() });
	let scenery = SceneryBake::new(project);

	for y in 0..height {
		for x in 0..width {
			let stack = project.cell(x, y).expect("cell in range");
			let mut composed = match (stack[LAYER_WATER], stack[LAYER_GROUND], canonical_water) {
				(Some(_), Some(ground), Some(canon)) => {
					let mut canonical = *stack;
					canonical[LAYER_WATER] = Some(canon);
					canonical[LAYER_GROUND] = Some(ground);
					project.compose_stack(&canonical)
				}
				_ => project.compose_stack(stack),
			};
			scenery.paint(&mut composed, x, y);
			minimap.push(composed[CENTER_PIXEL]);
			let index = match index_of.get(&composed) {
				Some(&index) => index,
				None => {
					if index_of.len() >= MAX_BAKED_TILES {
						return Err(format!("bake: over the {MAX_BAKED_TILES}-tile budget at cell ({x},{y})",));
					}
					let index = index_of.len() as u16;
					tiles.extend_from_slice(&composed);
					pass_table.push(stack_pass(project, x, y)?);
					index_of.insert(composed, index);
					index
				}
			};
			bigmap.push(index);
		}
	}

	Ok(WrlFile {
		header: WRL_HEADER.to_vec(),
		width,
		height,
		minimap,
		bigmap,
		tile_count: index_of.len() as u16,
		tiles,
		palette: project.palette.clone(),
		pass_table,
	})
}

/// One resolved placement: the piece, where its sprite lands in map pixels, and
/// how its ink meets the scenery already there.
struct Placed<'a> {
	piece: &'a crate::scenery::SceneryPiece,
	origin: (i32, i32),
	blend: SceneryBlend,
	/// The piece's inferred relief, one byte per sprite pixel - what `higher`
	/// compares two placements by. Worked out once per placement here rather
	/// than per cell: a bake visits every cell the sprite touches, and the field
	/// is a blur over the whole sprite.
	height: Vec<u8>,
}

/// What one bake needs to paint scenery, worked out once:
///
/// * a [`ShadeTable`] per distinct alpha the placed pieces use, because a
///   nearest-colour search per shadow pixel would dominate the export;
/// * the placements themselves, resolved in placement order;
/// * the palette's brightness table, which is what `brighter` / `darker` sort
///   two inks by.
///
/// All empty for a map with no scenery, and that map bakes exactly as it did
/// before scenery existed.
struct SceneryBake<'a> {
	tables: Vec<(u8, ShadeTable)>,
	placed: Vec<Placed<'a>>,
	brightness: [u8; 256],
}

impl<'a> SceneryBake<'a> {
	fn new(project: &'a Project) -> Self {
		let brightness = crate::scenery::brightness_table(&project.palette);
		let wants_height = project.scenery.iter().any(|s| s.blend == SceneryBlend::Higher);
		let mut seen = [false; 256];
		let mut placed = Vec::new();
		for spot in &project.scenery {
			let Some(piece) = project.scenery_piece(spot) else { continue };
			for &alpha in &piece.sprite.shade {
				seen[alpha as usize] = true;
			}
			// Every placement carries its relief once *any* of them blends by it:
			// the one doing the comparing needs the height of whatever it lands
			// on, whatever mode that one was placed with. A map with no `higher`
			// placement pays nothing, and the field is a blur over the whole
			// sprite, so it is worth the check.
			let height = if wants_height { piece.height_field(&brightness) } else { Vec::new() };
			placed.push(Placed { piece, origin: piece.sprite_origin(spot), blend: spot.blend, height });
		}
		let tables =
			(1..=255u8).filter(|&a| seen[a as usize]).map(|a| (a, ShadeTable::build(&project.palette, a))).collect();
		Self { tables, placed, brightness }
	}

	/// Paint every placement that reaches cell `(x, y)` over `composed`.
	fn paint(&self, composed: &mut [u8; TILE_DATA_SIZE], x: u16, y: u16) {
		if self.placed.is_empty() {
			return;
		}
		let (cx, cy) = (x as i32 * TILE_SIZE as i32, y as i32 * TILE_SIZE as i32);
		self.paint_cell(&self.placed, composed, cx, cy);
	}

	/// Composite `placed` over one cell whose top-left is map pixel `(cx, cy)`.
	///
	/// **Shadows are one layer, and it lies under every body.** Two objects whose
	/// shadows overlap darken the ground once, to the depth of the *strongest* of
	/// them, rather than each darkening what the last one left - which compounded
	/// into a black blot wherever a stand of trees was planted close together.
	/// The merge is a `max`, so a shared shadow reads exactly as deep as either
	/// object's own, and an object's ink is never dimmed by its neighbour's
	/// shadow.
	///
	/// Bodies then paint in placement order, each meeting the one under it by its
	/// own [`SceneryBlend`]: `normal` covers, `brighter` and `darker` keep
	/// whichever of the two inks is lighter or darker, and `higher` keeps the ink
	/// of whichever object stands taller there. Only *scenery* counts as
	/// underneath - a pixel no earlier placement's body covered takes the
	/// placement's own ink whatever the mode says, because the ground is not in
	/// the comparison.
	fn paint_cell(&self, placed: &[Placed], composed: &mut [u8; TILE_DATA_SIZE], cx: i32, cy: i32) {
		let mut shade = [0u8; TILE_DATA_SIZE];
		let mut shadowed = false;
		for p in placed {
			each_texel(p.piece, p.origin, cx, cy, |at, _, _, s| {
				if s > shade[at] {
					shade[at] = s;
					shadowed = true;
				}
			});
		}
		if shadowed {
			for (at, &alpha) in shade.iter().enumerate() {
				if alpha == 0 {
					continue;
				}
				if let Some((_, table)) = self.tables.iter().find(|(a, _)| *a == alpha) {
					composed[at] = table.apply(composed[at]);
				}
			}
		}
		// The scenery so far, kept apart from the composed ground so a mode can
		// tell "no scenery here" from "scenery that happens to be dark", and
		// beside it how high the object holding each of those inks stands - what
		// `higher` compares against. The height follows the ink: a pixel whose
		// ink an earlier placement kept keeps that placement's height too, so the
		// pair never describes two different objects.
		let mut ink = [0u8; TILE_DATA_SIZE];
		let mut high = [0u8; TILE_DATA_SIZE];
		for p in placed {
			each_texel(p.piece, p.origin, cx, cy, |at, si, body, _| {
				if body == 0 {
					return;
				}
				let h = p.height.get(si).copied().unwrap_or(0);
				if ink[at] == 0 {
					(ink[at], high[at]) = (body, h);
					return;
				}
				let kept = p.blend.pick(body, ink[at], h, high[at], &self.brightness);
				// The renderer settles `higher` with a depth test, and a depth
				// write happens exactly when that test passes - so a `higher`
				// placement records its height only where it won, and every other
				// mode records it outright. The screen and the export have to
				// agree about the height a *later* placement then compares with,
				// not only about the ink.
				if p.blend != SceneryBlend::Higher || kept == body {
					high[at] = h;
				}
				ink[at] = kept;
			});
		}
		for (at, &body) in ink.iter().enumerate() {
			if body != 0 {
				composed[at] = body;
			}
		}
	}
}

/// Visit the texels of `piece`'s sprite that land inside the cell at `(cx, cy)`,
/// as `(offset into the cell, index into the sprite's planes, body, shade)`.
fn each_texel(
	piece: &crate::scenery::SceneryPiece,
	(ox, oy): (i32, i32),
	cx: i32,
	cy: i32,
	mut f: impl FnMut(usize, usize, u8, u8),
) {
	let x0 = ox.max(cx);
	let y0 = oy.max(cy);
	let x1 = (ox + piece.sprite.width as i32).min(cx + TILE_SIZE as i32);
	let y1 = (oy + piece.sprite.height as i32).min(cy + TILE_SIZE as i32);
	for py in y0..y1 {
		for px in x0..x1 {
			let (lx, ly) = (px - ox, py - oy);
			let (body, shade) = piece.texel(lx, ly);
			let si = ly as usize * piece.sprite.width as usize + lx as usize;
			f((py - cy) as usize * TILE_SIZE + (px - cx) as usize, si, body, shade);
		}
	}
}

/// Pass value of a cell = its stack-top tile's pack pass entry
/// (`Project::pass_at`; missing pass data is an export error).
fn stack_pass(project: &Project, x: u16, y: u16) -> Result<u8, String> {
	project.pass_at(x, y).ok_or_else(|| format!("bake: stack top at ({x},{y}) has no pass data (tiles.pass.json)"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::scenery::{SceneryPiece, Sprite};

	/// A grey ramp, so a darkening lands on a lower index and the arithmetic is
	/// readable.
	fn palette() -> Vec<u8> {
		(0..256u32).flat_map(|i| [i as u8, i as u8, i as u8]).collect()
	}

	/// A 1-row piece from `(body, shade)` pairs. `name`'s first word is the
	/// family, so tests that mean two unrelated objects must name them apart.
	fn piece(name: &str, row: &[(u8, u8)]) -> SceneryPiece {
		SceneryPiece {
			id: name.to_lowercase(),
			name: name.into(),
			family: crate::scenery::piece_family(name),
			transformable: Default::default(),
			peak: None,
			sunken: None,
			scarp: None,
			height: None,
			cells_w: 1,
			cells_h: 1,
			pass: vec![0],
			sprite: Sprite {
				width: row.len() as u16,
				height: 1,
				origin_x: 0,
				origin_y: 0,
				body: row.iter().map(|&(b, _)| b).collect(),
				shade: row.iter().map(|&(_, s)| s).collect(),
			},
			user: true,
		}
	}

	/// The bake `SceneryBake::new` would build for these placements, in order.
	fn scene<'a>(
		alphas: &[(u8, ShadeTable)],
		spots: Vec<(&'a SceneryPiece, (i32, i32), SceneryBlend)>,
	) -> SceneryBake<'a> {
		let placed = spots
			.into_iter()
			.map(|(piece, origin, blend)| Placed {
				piece,
				origin,
				blend,
				height: piece.height_field(&crate::scenery::brightness_table(&palette())),
			})
			.collect();
		SceneryBake { tables: alphas.to_vec(), placed, brightness: crate::scenery::brightness_table(&palette()) }
	}

	/// Run one cell through the bake and hand back the composed tile.
	fn paint(bake: &SceneryBake, ground: u8) -> [u8; TILE_DATA_SIZE] {
		let mut composed = [ground; TILE_DATA_SIZE];
		bake.paint_cell(&bake.placed, &mut composed, 0, 0);
		composed
	}

	/// **Two shadows over one another darken the ground once.** Overlapping
	/// placements used to apply their shade planes in sequence, so a stand of
	/// trees planted close together compounded into a blot; the merged layer
	/// takes the strongest alpha and applies it a single time.
	#[test]
	fn overlapping_shadows_merge_instead_of_compounding() {
		let palette = palette();
		let ground = 200u8;
		let once = ShadeTable::build(&palette, 128).apply(ground);
		assert!(once < ground, "a shadow darkens the ground it falls on");

		// Two 2px shadows at x = 0 and x = 1: pixel 1 carries both.
		let (a, b) = (piece("A", &[(0, 128), (0, 128)]), piece("B", &[(0, 128), (0, 128)]));
		let bake = scene(
			&[(128, ShadeTable::build(&palette, 128))],
			vec![(&a, (0, 0), SceneryBlend::Normal), (&b, (1, 0), SceneryBlend::Normal)],
		);
		let composed = paint(&bake, ground);
		assert_eq!(composed[0], once, "one shadow");
		assert_eq!(composed[1], once, "two shadows, same depth - not darker");
		assert_eq!(composed[2], once, "one shadow");
		assert_eq!(composed[3], ground, "past both, the ground is untouched");
		// Not a vacuous test: applying the same shadow twice really is darker.
		assert!(ShadeTable::build(&palette, 128).apply(once) < once, "twice would be darker still");
	}

	/// The merged shadow lies **under** every body: one object's ink is never
	/// dimmed by its neighbour's shadow, which is what "merge shadows below
	/// them" means. A later body still covers an earlier one.
	#[test]
	fn a_body_is_never_dimmed_by_another_objects_shadow() {
		let palette = palette();
		// `a` is solid ink at x = 0..2, `b` casts shadow over both, and `c` puts
		// its own ink on top of the second.
		let a = piece("A", &[(90, 0), (90, 0)]);
		let b = piece("B", &[(0, 128), (0, 128)]);
		let c = piece("C", &[(70, 0)]);
		let bake = scene(
			&[(128, ShadeTable::build(&palette, 128))],
			vec![
				(&a, (0, 0), SceneryBlend::Normal),
				(&b, (0, 0), SceneryBlend::Normal),
				(&c, (1, 0), SceneryBlend::Normal),
			],
		);
		let composed = paint(&bake, 200);
		assert_eq!(composed[0], 90, "the shadow falls under the ink, not over it");
		assert_eq!(composed[1], 70, "and a later body still covers an earlier one");
	}

	/// A cell no placement reaches is left exactly as the stack composed it.
	#[test]
	fn a_cell_with_nothing_over_it_is_untouched() {
		let bake = scene(&[], Vec::new());
		assert!(paint(&bake, 17).iter().all(|&p| p == 17));
	}

	/// **A blend mode only ever picks one of the two inks**, and only where the
	/// two are both scenery: over bare ground every mode paints the placement's
	/// own ink, so the ground is never part of the comparison.
	#[test]
	fn a_blend_mode_picks_between_two_scenery_inks_only() {
		let ground = 200u8;
		let (dark, light) = (piece("Dark", &[(40, 0), (40, 0)]), piece("Light", &[(90, 0)]));
		for (mode, over_both) in [
			(SceneryBlend::Normal, 90),
			(SceneryBlend::Brighter, 90),
			(SceneryBlend::Darker, 40),
			// The two stand equally high, and a tie keeps the newer placement.
			(SceneryBlend::Higher, 90),
		] {
			let bake = scene(&[], vec![(&dark, (0, 0), SceneryBlend::Normal), (&light, (0, 0), mode)]);
			let composed = paint(&bake, ground);
			assert_eq!(composed[0], over_both, "{mode:?}: the two inks meet");
			assert_eq!(composed[1], 40, "{mode:?}: only the earlier piece is here");
			assert_eq!(composed[2], ground, "{mode:?}: no scenery at all");
		}
	}

	/// `higher` keeps the ink of whichever object stands taller, whichever of the
	/// two was placed first - and it is the *object* that is compared, not the
	/// order: a small hill dropped over a mountain's flank does not cover it.
	#[test]
	fn higher_keeps_the_taller_object() {
		let tall = SceneryPiece { peak: Some(200), ..piece("Tall", &[(50, 0)]) };
		let low = SceneryPiece { peak: Some(20), ..piece("Low", &[(90, 0)]) };
		let over = |first: &SceneryPiece, second: &SceneryPiece| {
			let bake = scene(&[], vec![(first, (0, 0), SceneryBlend::Normal), (second, (0, 0), SceneryBlend::Higher)]);
			paint(&bake, 200)[0]
		};
		assert_eq!(over(&low, &tall), 50, "the tall one placed second covers the low one");
		assert_eq!(over(&tall, &low), 50, "and placed first it still shows through the low one");
	}

	/// The height layer follows the **ink**, so a third placement compares
	/// against the object whose ink is actually there - not against whatever was
	/// painted last and lost.
	#[test]
	fn a_losing_placement_does_not_leave_its_height_behind() {
		let tall = SceneryPiece { peak: Some(200), ..piece("Tall", &[(50, 0)]) };
		let low = SceneryPiece { peak: Some(20), ..piece("Low", &[(90, 0)]) };
		let mid = SceneryPiece { peak: Some(100), ..piece("Mid", &[(70, 0)]) };
		// `low` loses to `tall` and records nothing; `mid` then meets `tall`'s
		// own height and loses too.
		let bake = scene(
			&[],
			vec![
				(&tall, (0, 0), SceneryBlend::Normal),
				(&low, (0, 0), SceneryBlend::Higher),
				(&mid, (0, 0), SceneryBlend::Higher),
			],
		);
		assert_eq!(paint(&bake, 200)[0], 50, "the mountain is still the tallest thing here");
	}

	/// The mode belongs to the placement being painted, not to the pack or the
	/// map: three placements in a row each apply their own.
	#[test]
	fn each_placement_carries_its_own_mode() {
		let (a, b, c) = (piece("A", &[(50, 0)]), piece("B", &[(90, 0)]), piece("C", &[(70, 0)]));
		// 50 laid down, then 90 kept darker (-> 50), then 70 kept brighter (-> 70).
		let bake = scene(
			&[],
			vec![
				(&a, (0, 0), SceneryBlend::Normal),
				(&b, (0, 0), SceneryBlend::Darker),
				(&c, (0, 0), SceneryBlend::Brighter),
			],
		);
		assert_eq!(paint(&bake, 200)[0], 70);
		// ...and with the last two swapped: 50, then 70 brighter (-> 70), then 90
		// darker (-> 70).
		let bake = scene(
			&[],
			vec![
				(&a, (0, 0), SceneryBlend::Normal),
				(&c, (0, 0), SceneryBlend::Brighter),
				(&b, (0, 0), SceneryBlend::Darker),
			],
		);
		assert_eq!(paint(&bake, 200)[0], 70);
	}
}
