use crossbeam;
use image::Rgba;

use crate::image_to_wrl::log;

#[inline(always)]
fn pixel_idx(x: u32, y: u32, width: u32) -> usize {
    (y * width + x) as usize
}

fn find_closest_palette_color(color_f32: &[f32; 4], palette: &[Rgba<u8>]) -> u8 {
    let mut min_dist_sq = f32::MAX;
    let mut closest_index: u8 = 0;

    for (i, pal_color) in palette.iter().enumerate() {
        assert!(
            i <= u8::MAX as usize,
            "Palette has more than 256 colors, cannot use u8 for indices."
        );

        let pal_color_f32 = [
            pal_color[0] as f32,
            pal_color[1] as f32,
            pal_color[2] as f32,
            pal_color[3] as f32,
        ];

        let dist_sq = (color_f32[0] - pal_color_f32[0]).powi(2)
            + (color_f32[1] - pal_color_f32[1]).powi(2)
            + (color_f32[2] - pal_color_f32[2]).powi(2)
            + (color_f32[3] - pal_color_f32[3]).powi(2);

        if dist_sq < min_dist_sq {
            min_dist_sq = dist_sq;
            closest_index = i as u8;
        }
    }
    closest_index
}

/// Performs Floyd-Steinberg dithering on an RGBA image, processing in parallel stripes.
/// WARNING: This will produce visibly incorrect dithering due to inter-stripe dependencies
/// (errors are not propagated across stripe boundaries). This is for demonstration of
/// non-overlapping parallel processing, not a correct dither implementation.
pub fn floyd_steinberg(
    original_rgba_img: &[u8],
    width: u32,
    height: u32,
    final_palette: &[Rgba<u8>],
    num_stripes: usize,
) -> Result<Vec<u8>, String> {
    // Assertions for input validity
    assert_eq!(
        original_rgba_img.len(),
        (width * height * 4) as usize,
        "Image data length ({}) does not match width ({}) * height ({}) * 4. Expected {}.",
        original_rgba_img.len(),
        width,
        height,
        (width * height * 4)
    );
    assert!(
        !final_palette.is_empty(),
        "The final_palette cannot be empty. It must contain at least one color."
    );
    assert!(
        final_palette.len() <= u8::MAX as usize + 1, // Max 256 colors (0-255) for u8 indices
        "Palette has {} colors, but indices are u8 (max 256 colors).",
        final_palette.len()
    );

    // Create a mutable copy of the image data, converting u8 channels to f32.
    // This `current_image_data` will be the working buffer where errors are diffused.
    let mut current_image_data: Vec<[f32; 4]> = Vec::with_capacity((width * height) as usize);
    for i in 0..(width * height) as usize {
        let orig_idx = i * 4;
        current_image_data.push([
            original_rgba_img[orig_idx] as f32,
            original_rgba_img[orig_idx + 1] as f32,
            original_rgba_img[orig_idx + 2] as f32,
            original_rgba_img[orig_idx + 3] as f32,
        ]);
    }

    // This will store the final dithered palette indices.
    let mut dithered_indices: Vec<u8> = vec![0u8; (width * height) as usize];

    // Calculate stripe height and number of actual stripes
    let num_actual_stripes = num_stripes.min(height as usize); // Don't create more stripes than rows
    let stripe_height = (height as f64 / num_actual_stripes as f64).ceil() as u32;

    //// println!("Attempting parallel Floyd-Steinberg with {} stripes, each ~{} rows high.", num_actual_stripes, stripe_height);
    //// println!("WARNING: This will produce visibly incorrect dithering due to inter-stripe dependencies.");

    log::start_progress();

    // Use crossbeam::scope to allow mutable access to parts of `current_image_data`
    // and `dithered_indices`. The scope guarantees that threads finish before `data` goes out of scope.
    crossbeam::scope(|s| {
        let mut current_data_slice = &mut current_image_data[..];
        let mut dithered_indices_slice = &mut dithered_indices[..];

        for i in 0..num_actual_stripes {
            let start_row_global = i as u32 * stripe_height;
            let end_row_global = (start_row_global + stripe_height).min(height);

            // Calculate length of this stripe's data in elements
            let rows_in_stripe = end_row_global - start_row_global;
            let chunk_len_elements = (rows_in_stripe * width) as usize;

            if chunk_len_elements == 0 {
                // This can happen if start_row_global == end_row_global, or no more data
                continue;
            }

            // Split the overall mutable slices into the current stripe's portion and the remainder.
            let (this_stripe_data, next_data_slice) =
                current_data_slice.split_at_mut(chunk_len_elements);
            let (this_stripe_indices, next_indices_slice) =
                dithered_indices_slice.split_at_mut(chunk_len_elements);

            // Update the 'remainder' slices for the next iteration
            current_data_slice = next_data_slice;
            dithered_indices_slice = next_indices_slice;

            // `palette_ref` can be shared as an immutable reference (`&`) since it's not modified.
            // Wrap in `Arc` if palette itself is dynamically allocated and needs shared ownership,
            // but for a slice, a direct ref is fine.
            let palette_ref = final_palette;
            let stripe_width = width; // Capture width for the closure
            let stripe_height_local = rows_in_stripe; // Capture local height for the closure

            s.spawn(move |_| {
                for y_local in 0..stripe_height_local {
                    // Iterate over rows within this stripe
                    for x in 0..stripe_width {
                        // Iterate over columns
                        let idx_local = pixel_idx(x, y_local, stripe_width);

                        // `old_pixel_f32` comes from the current, error-accumulated value
                        let old_pixel_f32 = this_stripe_data[idx_local];

                        // Find closest palette color
                        let closest_palette_index =
                            find_closest_palette_color(&old_pixel_f32, palette_ref);
                        this_stripe_indices[idx_local] = closest_palette_index; // Store index in the output buffer

                        // Quantized color
                        let pal = palette_ref[closest_palette_index as usize];
                        let new_pixel_f32 =
                            [pal[0] as f32, pal[1] as f32, pal[2] as f32, pal[3] as f32];

                        // Error calculation
                        let error = [
                            old_pixel_f32[0] - new_pixel_f32[0],
                            old_pixel_f32[1] - new_pixel_f32[1],
                            old_pixel_f32[2] - new_pixel_f32[2],
                            old_pixel_f32[3] - new_pixel_f32[3],
                        ];

                        // Error diffusion within the current stripe's boundaries
                        // Apply error to neighbors in `this_stripe_data`
                        // Right neighbor
                        if x + 1 < stripe_width {
                            let next_idx = pixel_idx(x + 1, y_local, stripe_width);
                            for c in 0..4 {
                                this_stripe_data[next_idx][c] += error[c] * 7.0 / 16.0;
                            }
                        }
                        // Bottom-left neighbor
                        if x > 0 && y_local + 1 < stripe_height_local {
                            let next_idx = pixel_idx(x - 1, y_local + 1, stripe_width);
                            for c in 0..4 {
                                this_stripe_data[next_idx][c] += error[c] * 3.0 / 16.0;
                            }
                        }
                        // Bottom neighbor
                        if y_local + 1 < stripe_height_local {
                            let next_idx = pixel_idx(x, y_local + 1, stripe_width);
                            for c in 0..4 {
                                this_stripe_data[next_idx][c] += error[c] * 5.0 / 16.0;
                            }
                        }
                        // Bottom-right neighbor
                        if x + 1 < stripe_width && y_local + 1 < stripe_height_local {
                            let next_idx = pixel_idx(x + 1, y_local + 1, stripe_width);
                            for c in 0..4 {
                                this_stripe_data[next_idx][c] += error[c] * 1.0 / 16.0;
                            }
                        }
                    }

                    if i == 0 {
                        log::progress(y_local as usize + 1, stripe_height_local as usize);
                    }
                }
            });
        }
    })
    .unwrap(); // Propagate any panics from spawned threads

    log::line_up();

    Ok(dithered_indices)
}

pub fn stucki(
    original_rgba_img: &[u8],
    width: u32,
    height: u32,
    final_palette: &[Rgba<u8>],
    num_stripes: usize,
) -> Result<Vec<u8>, String> {
    // Assertions for input validity
    assert_eq!(
        original_rgba_img.len(),
        (width * height * 4) as usize,
        "Image data length ({}) does not match width ({}) * height ({}) * 4. Expected {}.",
        original_rgba_img.len(),
        width,
        height,
        (width * height * 4)
    );
    assert!(
        !final_palette.is_empty(),
        "The final_palette cannot be empty. It must contain at least one color."
    );
    assert!(
        final_palette.len() <= u8::MAX as usize + 1, // Max 256 colors (0-255) for u8 indices
        "Palette has {} colors, but indices are u8 (max 256 colors).",
        final_palette.len()
    );

    // Create a mutable copy of the image data, converting u8 channels to f32.
    // This `current_image_data` will be the working buffer where errors are diffused.
    let mut current_image_data: Vec<[f32; 4]> = Vec::with_capacity((width * height) as usize);
    for i in 0..(width * height) as usize {
        let orig_idx = i * 4;
        current_image_data.push([
            original_rgba_img[orig_idx] as f32,
            original_rgba_img[orig_idx + 1] as f32,
            original_rgba_img[orig_idx + 2] as f32,
            original_rgba_img[orig_idx + 3] as f32,
        ]);
    }

    // This will store the final dithered palette indices.
    let mut dithered_indices: Vec<u8> = vec![0u8; (width * height) as usize];

    // Calculate stripe height and number of actual stripes
    let num_actual_stripes = num_stripes.min(height as usize); // Don't create more stripes than rows
    let stripe_height = (height as f64 / num_actual_stripes as f64).ceil() as u32;

    log::start_progress();

    // Use crossbeam::scope to allow mutable access to parts of `current_image_data`
    // and `dithered_indices`. The scope guarantees that threads finish before `data` goes out of scope.
    crossbeam::scope(|s| {
        let mut current_data_slice = &mut current_image_data[..];
        let mut dithered_indices_slice = &mut dithered_indices[..];
        for i in 0..num_actual_stripes {
            let start_row_global = i as u32 * stripe_height;
            let end_row_global = (start_row_global + stripe_height).min(height);

            // Calculate length of this stripe's data in elements
            let rows_in_stripe = end_row_global - start_row_global;
            let chunk_len_elements = (rows_in_stripe * width) as usize;

            if chunk_len_elements == 0 {
                // This can happen if start_row_global == end_row_global, or no more data
                continue;
            }

            // Split the overall mutable slices into the current stripe's portion and the remainder.
            let (this_stripe_data, next_data_slice) =
                current_data_slice.split_at_mut(chunk_len_elements);
            let (this_stripe_indices, next_indices_slice) =
                dithered_indices_slice.split_at_mut(chunk_len_elements);

            // Update the 'remainder' slices for the next iteration
            current_data_slice = next_data_slice;
            dithered_indices_slice = next_indices_slice;

            // `palette_ref` can be shared as an immutable reference (`&`) since it's not modified.
            // Wrap in `Arc` if palette itself is dynamically allocated and needs shared ownership,
            // but for a slice, a direct ref is fine.
            let palette_ref = final_palette;
            let stripe_width = width; // Capture width for the closure
            let stripe_height_local = rows_in_stripe; // Capture local height for the closure

            s.spawn(move |_| {
                for y_local in 0..stripe_height_local {
                    // Iterate over rows within this stripe
                    for x in 0..stripe_width {
                        // Iterate over columns
                        let idx_local = pixel_idx(x, y_local, stripe_width);

                        // `old_pixel_f32` comes from the current, error-accumulated value
                        let old_pixel_f32 = this_stripe_data[idx_local];

                        // Find closest palette color
                        let closest_palette_index =
                            find_closest_palette_color(&old_pixel_f32, palette_ref);
                        this_stripe_indices[idx_local] = closest_palette_index; // Store index in the output buffer

                        // Quantized color
                        let pal = palette_ref[closest_palette_index as usize];
                        let new_pixel_f32 =
                            [pal[0] as f32, pal[1] as f32, pal[2] as f32, pal[3] as f32];
                        // Error calculation
                        let error = [
                            old_pixel_f32[0] - new_pixel_f32[0],
                            old_pixel_f32[1] - new_pixel_f32[1],
                            old_pixel_f32[2] - new_pixel_f32[2],
                            old_pixel_f32[3] - new_pixel_f32[3],
                        ];
                        // Error diffusion within the current stripe's boundaries
                        // Apply error to neighbors in `this_stripe_data`
                        // Right neighbor
                        if x + 1 < stripe_width {
                            let next_idx = pixel_idx(x + 1, y_local, stripe_width);
                            for c in 0..4 {
                                this_stripe_data[next_idx][c] += error[c] * 8.0 / 42.0;
                            }
                        }
                        // Bottom-left neighbor
                        if x > 0 && y_local + 1 < stripe_height_local {
                            let next_idx = pixel_idx(x - 1, y_local + 1, stripe_width);
                            for c in 0..4 {
                                this_stripe_data[next_idx][c] += error[c] * 4.0 / 42.0;
                            }
                        }
                        // Bottom neighbor
                        if y_local + 1 < stripe_height_local {
                            let next_idx = pixel_idx(x, y_local + 1, stripe_width);
                            for c in 0..4 {
                                this_stripe_data[next_idx][c] += error[c] * 8.0 / 42.0;
                            }
                        }
                        // Bottom-right neighbor
                        if x + 1 < stripe_width && y_local + 1 < stripe_height_local {
                            let next_idx = pixel_idx(x + 1, y_local + 1, stripe_width);
                            for c in 0..4 {
                                this_stripe_data[next_idx][c] += error[c] * 2.0 / 42.0;
                            }
                        }
                    }
                    if i == 0 {
                        log::progress(y_local as usize + 1, stripe_height_local as usize);
                    }
                }
            });
        }
    })
    .unwrap(); // Propagate any panics from spawned threads
    log::line_up();
    Ok(dithered_indices)
}
