//! Build the shipped stock templates (`resources/assets/templates`) from each
//! pack's mined formation patterns (`tiles.patterns.json`) - the largest few
//! formations per pack make good starter templates. Deterministic; re-run
//! after pack data changes:
//!
//! ```sh
//! cargo run -p map-core --example make_stock_templates
//! ```

use map_core::{Template, TilePack};

fn main() {
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
	let out = root.join("assets/templates");
	let mut made = 0;
	for pack_name in ["GREEN", "CRATER", "DESERT", "SNOW", "SNOW_DARK"] {
		let pack = match TilePack::load(&root.join("assets/tilepacks"), pack_name) {
			Ok(p) => p,
			Err(e) => {
				eprintln!("skip {pack_name}: {e}");
				continue;
			}
		};
		// One pack per generated template → the pack's own subdir.
		let dir = out.join(pack_name);
		std::fs::create_dir_all(&dir).expect("create pack template dir");
		// Biggest formations first (most cells), name ties broken by order.
		let mut patterns: Vec<_> = pack.patterns.iter().collect();
		patterns.sort_by_key(|p| std::cmp::Reverse(p.cells.iter().flatten().count()));
		for (i, p) in patterns.iter().take(2).enumerate() {
			let cells: Vec<String> =
				p.cells.iter().map(|c| c.map(|t| pack.ids[t as usize].clone()).unwrap_or_default()).collect();
			let name = format!("{}-formation-{}", pack_name.to_lowercase(), i + 1);
			let template = Template {
				name: name.clone(),
				width: p.width,
				height: p.height,
				uses: vec![(pack.name.clone(), pack.version.clone())],
				cells,
			};
			template.save(&dir.join(format!("{name}.json"))).expect("write template");
			println!("{name}: {}x{} ({} tiles)", p.width, p.height, p.cells.iter().flatten().count());
			made += 1;
		}
	}
	println!("wrote {made} stock templates to {}", out.display());
}
