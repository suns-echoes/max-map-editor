use image::Rgba;
use std::error::Error;
use std::fs;
use std::path::Path;

use crate::app_state;

fn parse_hex_color(hex_str: &str) -> Result<Rgba<u8>, Box<dyn Error>> {
    if !hex_str.starts_with('#') {
        return Err("Hex color string must start with '#'".into());
    }
    let hex_digits = &hex_str[1..];

    let len = hex_digits.len();
    let (r, g, b, a);

    if len == 6 {
        // #RRGGBB format
        r = u8::from_str_radix(&hex_digits[0..2], 16)?;
        g = u8::from_str_radix(&hex_digits[2..4], 16)?;
        b = u8::from_str_radix(&hex_digits[4..6], 16)?;
        a = 255; // Default to opaque
    } else if len == 8 {
        // #RRGGBBAA format
        r = u8::from_str_radix(&hex_digits[0..2], 16)?;
        g = u8::from_str_radix(&hex_digits[2..4], 16)?;
        b = u8::from_str_radix(&hex_digits[4..6], 16)?;
        a = u8::from_str_radix(&hex_digits[6..8], 16)?;
    } else {
        return Err(format!(
            "Invalid hex color string length: {}. Expected 6 or 8 digits after '#'",
            len
        )
        .into());
    }

    Ok(Rgba([r, g, b, a]))
}

pub fn load() -> Result<Vec<Rgba<u8>>, Box<dyn Error>> {
	let resource_path = app_state::get_resource_path();
	let path = Path::new(&resource_path).join("internal").join("palette_slots.json");

    let file_content = fs::read_to_string(&path)
        .map_err(|e| format!("Could not read JSON file {:?}: {}", path, e))?;

    // Deserialize directly into a Vec of Strings
    let json_hex_colors: Vec<String> = serde_json::from_str(&file_content)
        .map_err(|e| format!("Could not parse JSON from {:?}: {}", path, e))?;

    let mut rgba_colors = Vec::new();
    for (i, hex_str) in json_hex_colors.into_iter().enumerate() {
        match parse_hex_color(&hex_str) {
            Ok(color) => rgba_colors.push(color),
            Err(e) => {
                eprintln!(
                    "Warning: Skipping invalid hex color string at index {}: '{}' - {}",
                    i, hex_str, e
                );
            }
        }
    }

    Ok(rgba_colors)
}

pub fn save_to_json_file(palette: &[Rgba<u8>]) -> Result<(), Box<dyn Error>> {
    let hex_colors: Vec<String> = palette
        .iter()
        .map(|color| {
            format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                color[0], color[1], color[2], color[3]
            )
        })
        .collect();

    let json_content = serde_json::to_string(&hex_colors)
        .map_err(|e| format!("Could not serialize palette to JSON: {}", e))?;

    fs::write("palette.json", json_content)
        .map_err(|e| format!("Could not write palette to file: {}", e))?;

    Ok(())
}
