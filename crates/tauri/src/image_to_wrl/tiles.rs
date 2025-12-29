pub fn from_indexed_image(
	image: &Vec<u8>,
	width: u32,
	height: u32,
	tile_size: u32,
) -> Vec<u8> {
	let mut tiles = Vec::new();
	let tiles_x = width / tile_size;
	let tiles_y = height / tile_size;

	for ty in 0..tiles_y {
		for tx in 0..tiles_x {
			for row in 0..tile_size {
				let y = ty * tile_size + row;
				let start = (y * width + tx * tile_size) as usize;
				let end = start + tile_size as usize;
				tiles.extend_from_slice(&image[start..end]);
			}
		}
	}
	tiles
}
