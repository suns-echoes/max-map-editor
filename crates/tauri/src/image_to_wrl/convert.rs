use image::Rgba;

pub fn vec_rgba_to_raw(colors: &[Rgba<u8>]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
	// Ensure the input vector has a length that is a multiple of 4
	if colors.len() % 4 != 0 {
		return Err("Input vector length must be a multiple of 4".into());
	}

	// Create a new vector to hold the raw bytes
	let mut raw_bytes = Vec::with_capacity(colors.len() * 4);

	// Iterate over the colors and convert each Rgba<u8> to raw bytes
	for color in colors {
		raw_bytes.extend_from_slice(&color.0);
	}

	Ok(raw_bytes)
}
