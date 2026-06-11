//! WRL export bake: flatten a project's cell stacks into a
//! flat WRL — `compose_cell` per cell, byte-identical tiles deduplicated,
//! pass from the stack-top tile's pack data, minimap from the composed
//! center pixel (the original game's own derivation is unknown — see
//! `docs/design/tileset-contract.md` §4).

use std::collections::HashMap;

use max_assets::wrl::{TILE_DATA_SIZE, WrlFile};

use crate::project::{LAYER_GROUND, LAYER_WATER, Project, TileRef, Transform};

/// `"WRL" 1 0` — the header all retail maps carry (demo maps use `DMO`).
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
	// phase is visually equivalent (animated noise) — the originals bake
	// exactly one phase per shore art. Re-pointing the water layer at the
	// water pack's tile 0 before composing keeps the dedup as tight as
	// Interplay's. Open-water cells keep their per-cell pattern tiles.
	let canonical_water = project.water_pack.map(|pack| TileRef { pack, tile: 0, transform: Transform::default() });

	for y in 0..height {
		for x in 0..width {
			let stack = project.cell(x, y).expect("cell in range");
			let composed = match (stack[LAYER_WATER], stack[LAYER_GROUND], canonical_water) {
				(Some(_), Some(ground), Some(canon)) => {
					let mut canonical = *stack;
					canonical[LAYER_WATER] = Some(canon);
					canonical[LAYER_GROUND] = Some(ground);
					project.compose_stack(&canonical)
				}
				_ => project.compose_stack(stack),
			};
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

/// Pass value of a cell = its stack-top tile's pack pass entry
/// (`Project::pass_at`; missing pass data is an export error).
fn stack_pass(project: &Project, x: u16, y: u16) -> Result<u8, String> {
	project.pass_at(x, y).ok_or_else(|| format!("bake: stack top at ({x},{y}) has no pass data (tiles.pass.json)"))
}
