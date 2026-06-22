//! Extract multi-tile formation patterns from the original maps into
//! `tiles.patterns.json` (worldgen). The match rules that would
//! describe obstruction adjacency in `tiles.match.json` were never authored,
//! so the originals are the ground truth: every connected formation of one
//! family (same 3-char id prefix; `tiles.props.json` types are definitive)
//! becomes a pattern - irregular shapes keep `null` holes, duplicates
//! collapse.
//!
//! Usage: `cargo run -p map-core --example extract_patterns [ORIGINALS_DIR]`
//! (default `testdata/originals`; the gitignored fixtures from
//! `tools/fetch-testdata.sh`). Writes `resources/assets/<PACK>/tiles.patterns.json`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use map_core::{TileKind, TilePack, family_of};
use max_assets::wrl::read_wrl_file;

/// One extracted formation: family cells in a bounding box, `None` = hole.
#[derive(PartialEq, Eq, Hash)]
struct Pattern {
	w: usize,
	h: usize,
	cells: Vec<Option<String>>,
}

fn main() {
	let originals = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("testdata/originals"));
	if !originals.is_dir() {
		eprintln!("no originals at {} - run tools/fetch-testdata.sh first", originals.display());
		std::process::exit(2);
	}
	let assets = Path::new("resources/assets/tilepacks");
	for (pack_name, originals_prefix) in
		[("CRATER", "CRATER"), ("DESERT", "DESERT"), ("GREEN", "GREEN"), ("SNOW", "SNOW"), ("SNOW_DARK", "SNOW")]
	{
		// SNOW_DARK has no original maps; where its tiles are pixel-identical
		// to ones the SNOW originals use, formations carry over for free.
		extract_pack(assets, &originals, pack_name, originals_prefix);
	}
}

fn extract_pack(assets: &Path, originals: &Path, pack_name: &str, originals_prefix: &str) {
	let pack = TilePack::load(assets, pack_name).expect(pack_name);

	// Pixel-exact lookup: original WRL tile data → pack tile id.
	let mut by_pixels: HashMap<&[u8], u16> = HashMap::new();
	for i in 0..pack.tile_count() {
		by_pixels.insert(pack.tile_pixels(i), i);
	}

	// Families worth patterns: typed OBSTRUCTION or LAND, *without*
	// variants (hasVariants families are interchangeable singles - the
	// randomizer's pool, not formations).
	let wanted = |family: &str| -> bool {
		pack.props
			.get(family)
			.is_some_and(|p| !p.has_variants && matches!(p.kind, Some(TileKind::Obstruction) | Some(TileKind::Land)))
	};

	let mut patterns: Vec<Pattern> = Vec::new();
	let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
	let (mut cells_total, mut cells_unmatched) = (0usize, 0usize);

	for map_no in 1..=6 {
		let path = originals.join(format!("{originals_prefix}_{map_no}.WRL"));
		let wrl = match read_wrl_file(&path) {
			Ok(wrl) => wrl,
			Err(e) => {
				eprintln!("skip {}: {e:?}", path.display());
				continue;
			}
		};
		let (w, h) = (wrl.width as usize, wrl.height as usize);
		// Cell → pack tile id (only for wanted families).
		let grid: Vec<Option<&str>> = wrl
			.bigmap
			.iter()
			.map(|&t| {
				cells_total += 1;
				let at = t as usize * 4096;
				let Some(&idx) = by_pixels.get(&wrl.tiles[at..at + 4096]) else {
					cells_unmatched += 1;
					return None;
				};
				let id = pack.ids[idx as usize].as_str();
				wanted(family_of(id)).then_some(id)
			})
			.collect();

		// 8-connected components per family - a formation is what reads as
		// one visual cluster, diagonal contact included.
		let mut comp_seen = vec![false; w * h];
		for start in 0..w * h {
			let Some(id) = grid[start] else { continue };
			if comp_seen[start] {
				continue;
			}
			let family = family_of(id);
			let mut cells: Vec<usize> = Vec::new();
			let mut stack = vec![start];
			comp_seen[start] = true;
			while let Some(i) = stack.pop() {
				cells.push(i);
				let (x, y) = ((i % w) as i32, (i / w) as i32);
				for dy in -1i32..=1 {
					for dx in -1i32..=1 {
						let (nx, ny) = (x + dx, y + dy);
						if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
							continue;
						}
						let j = ny as usize * w + nx as usize;
						if comp_seen[j] {
							continue;
						}
						if grid[j].is_some_and(|nid| family_of(nid) == family) {
							comp_seen[j] = true;
							stack.push(j);
						}
					}
				}
			}
			if cells.len() < 2 {
				continue; // a lone tile is not a formation
			}
			// Bounding box → row-major cells with None holes.
			let xs = cells.iter().map(|&i| i % w);
			let ys = cells.iter().map(|&i| i / w);
			let (x0, x1) = (xs.clone().min().unwrap(), xs.max().unwrap());
			let (y0, y1) = (ys.clone().min().unwrap(), ys.max().unwrap());
			let (pw, ph) = (x1 - x0 + 1, y1 - y0 + 1);
			let mut p = Pattern { w: pw, h: ph, cells: vec![None; pw * ph] };
			for &i in &cells {
				p.cells[(i / w - y0) * pw + (i % w - x0)] = Some(grid[i].unwrap().to_string());
			}
			// Dedupe exact repeats.
			let mut hasher = std::hash::DefaultHasher::new();
			use std::hash::{Hash, Hasher};
			p.hash(&mut hasher);
			if seen.insert(hasher.finish()) {
				patterns.push(p);
			}
		}
	}

	// Stable order: by family, then size, then cell content - re-runs and
	// map-iteration order can't shuffle the file.
	patterns.sort_by(|a, b| {
		let fam = |p: &Pattern| p.cells.iter().flatten().next().map(|id| family_of(id).to_string()).unwrap_or_default();
		(fam(a), a.cells.len(), format!("{:?}", a.cells)).cmp(&(fam(b), b.cells.len(), format!("{:?}", b.cells)))
	});

	// Report + write.
	let mut counts: HashMap<&str, usize> = HashMap::new();
	let mut max_dim = (0, 0);
	for p in &patterns {
		let fam = p.cells.iter().flatten().next().map(|id| family_of(id)).unwrap_or("");
		*counts.entry(fam).or_default() += 1;
		max_dim = (max_dim.0.max(p.w), max_dim.1.max(p.h));
	}
	let mut count_list: Vec<_> = counts.iter().collect();
	count_list.sort();
	eprintln!(
		"{pack_name}: {} patterns {:?}, max {}x{}, unmatched cells {}/{}",
		patterns.len(),
		count_list,
		max_dim.0,
		max_dim.1,
		cells_unmatched,
		cells_total,
	);

	let mut fam_no: HashMap<String, usize> = HashMap::new();
	let entries: Vec<String> = patterns
		.iter()
		.map(|p| {
			let fam = p.cells.iter().flatten().next().map(|id| family_of(id)).unwrap_or("").to_string();
			let n = fam_no.entry(fam.clone()).or_default();
			*n += 1;
			let rows: Vec<String> = (0..p.h)
				.map(|y| {
					let row: Vec<String> = (0..p.w)
						.map(|x| match &p.cells[y * p.w + x] {
							Some(id) => format!("\"{id}\""),
							None => "null".to_string(),
						})
						.collect();
					format!("\t\t\t[{}]", row.join(", "))
				})
				.collect();
			format!(
				"\t{{\n\t\t\"name\": \"{fam} {n}\",\n\t\t\"width\": {},\n\t\t\"height\": {},\n\t\t\"pattern\": [\n{}\n\t\t]\n\t}}",
				p.w,
				p.h,
				rows.join(",\n"),
			)
		})
		.collect();
	let json = format!("[\n{}\n]\n", entries.join(",\n"));
	let out = assets.join(pack_name).join("tiles.patterns.json");
	std::fs::write(&out, json).expect("write patterns");
	eprintln!("  wrote {}", out.display());
}
