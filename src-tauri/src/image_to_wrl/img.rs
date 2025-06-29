use std::path::Path;
use image::RgbaImage;
use std::error::Error;

pub fn load_image_as_rgba8<P: AsRef<Path>>(
    image_path: P,
) -> Result<(RgbaImage, u32, u32), Box<dyn Error>> {
    let path = image_path.as_ref();

    if !path.exists() {
        return Err(format!("Error: Image file not found at {:?}", path).into());
    }

    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    reader.no_limits(); // Disable decoding limits

    let img = reader.decode()?;
    let rgba_img = img.into_rgba8();
    let (width, height) = rgba_img.dimensions();

    Ok((rgba_img, width, height))
}

pub fn from_indexed(
    indexed_image: &[u8],
    width: u32,
    height: u32,
    palette: &[image::Rgba<u8>],
) -> Result<RgbaImage, String> {
    if indexed_image.len() != (width * height) as usize {
        return Err(format!(
            "Indexed image size ({}) does not match dimensions {}x{}.",
            indexed_image.len(),
            width,
            height
        ));
    }

    if palette.is_empty() {
        return Err("Palette cannot be empty.".to_string());
    }

    let mut rgba_image = RgbaImage::new(width, height);
    for (i, &index) in indexed_image.iter().enumerate() {
        if index as usize >= palette.len() {
            return Err(format!(
                "Index {} out of bounds for palette of size {}.",
                index,
                palette.len()
            ));
        }
        let color = palette[index as usize];
        rgba_image.put_pixel((i as u32 % width) as u32, (i as u32 / width) as u32, color);
    }

    Ok(rgba_image)
}
