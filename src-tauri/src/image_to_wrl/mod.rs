mod benchmark;
mod convert;
mod dither;
mod img;
mod log;
mod palette;
mod quantize_colors;

pub fn image_to_wrl(image_file_path: String) -> Result<(Vec<u8>, Vec<u8>), String> {
    //// let output_indexed_bmp_path = "quantized_no_dither.bmp";
    //// let output_dither_bmp_path = "quantized_dithered.bmp";
    //// let output_dither_bmp_path1 = "quantized_dithered_stucki.bmp";
    let max_iterations: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10); // Default to 1000 iterations if not specified

    let parallel_strips_count: usize = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1); // Limit to a maximum of 8 parallel strips

    let app_timer = benchmark::get_start_time();
    let mut local_timer: std::time::Instant;

    //
    // Print configuration
    //

    // log::hide_cursor();
    log::title("Image Quantization and Dithering Program");
    log::nl();
    log::param("Image file path", &image_file_path);
    log::param("Maximum iterations", &max_iterations.to_string());
    log::param("Using parallelization", &parallel_strips_count.to_string());
    //// log::nl();
    //// log::param("Output indexed BMP path", output_indexed_bmp_path);
    //// log::param("Output dithered BMP path", output_dither_bmp_path);
    log::nl();

    //
    // Load template palette
    //

    log::action("Loading template palette from JSON file...");

    local_timer = benchmark::get_start_time();

    let template_palette = match palette::load() {
        Ok(colors) => {
            log::ok(
                "template palette loaded successfully",
                Some(benchmark::get_elapsed_time(local_timer)),
            );
            colors
        }
        Err(e) => {
            log::error(&format!("failed to load template palette: {}", e));
            //// log::show_cursor();
            return Err(format!("Failed to load template palette: {}", e));
        }
    };

    //
    // Load the image
    //

    log::action("Loading image...");

    local_timer = benchmark::get_start_time();

    let (original_rgba_img, width, height) = match img::load_image_as_rgba8(image_file_path) {
        Ok((img_data, w, h)) => {
            log::ok(
                "image loaded successfully",
                Some(benchmark::get_elapsed_time(local_timer)),
            );
            (img_data, w, h)
        }
        Err(e) => {
            log::error(&format!("Failed to load image: {}", e));
            //// log::show_cursor();
            return Err(format!("Failed to load image: {}", e));
        }
    };

    //
    // Quantize colors, generate indexed image, and palette
    //

    log::action("Quantizing colors using K-means...");

    local_timer = benchmark::get_start_time();

    let k_colors = template_palette.len(); // Total palette size

    let final_palette = match quantize_colors::multithreaded(
        &original_rgba_img,
        k_colors,
        &template_palette,
        max_iterations,
        1.0,
    ) {
        Ok(results) => {
            log::ok(
                "quantization completed successfully",
                Some(benchmark::get_elapsed_time(local_timer)),
            );
            results
        }
        Err(e) => {
            log::error(&format!("Error during color quantization: {}", e));
            //// log::show_cursor();
            return Err(format!("Error during color quantization: {}", e));
        }
    };

	let raw_palette = match convert::vec_rgba_to_raw(&final_palette) {
		Ok(palette) => palette,
		Err(e) => {
			log::error(&format!("Failed to convert final palette to raw format: {}", e));
			//// log::show_cursor();
			return Err(format!("Failed to convert final palette to raw format: {}", e));
		}
	};


    //// //
    //// // Save final palette to JSON file
    //// //
	////
    //// log::action("Saving final palette to JSON file...");
	////
    //// local_timer = benchmark::get_start_time();
	////
    //// palette::save_to_json_file(&final_palette).unwrap_or_else(|e| {
    ////     log::error(&format!("Failed to save final palette: {}", e));
    ////     log::show_cursor();
    ////     return;
    //// });
	////
    //// log::ok(
    ////     "final palette saved successfully",
    ////     Some(benchmark::get_elapsed_time(local_timer)),
    //// );

    //// //
    //// // Parallel Floyd-Steinberg dithering
    //// //
    ////
    //// log::action("Dithering image using Floyd-Steinberg algorithm...");
    ////
    //// local_timer = benchmark::get_start_time();
    ////
    //// let indexed_dither_pixels = match dither::floyd_steinberg(
    ////     &original_rgba_img,
    ////     width,
    ////     height,
    ////     &final_palette,
    ////     parallel_strips_count,
    //// ) {
    ////     Ok(results) => {
    ////         log::ok(
    ////             "image dithering completed successfully",
    ////             Some(benchmark::get_elapsed_time(local_timer)),
    ////         );
    ////         results
    ////     }
    ////     Err(e) => {
    ////         log::error(&format!("Error during dithering: {}", e));
    ////         log::show_cursor();
    ////         return Err(format!("Error during dithering: {}", e));
    ////     }
    //// };

    //// //
    //// // Save the dithered indexed image as BMP
    //// //
    ////
    //// log::action("Saving dithered indexed image as BMP...");
    ////
    //// local_timer = benchmark::get_start_time();
    ////
    //// match image::from_indexed(&indexed_dither_pixels, width, height, &final_palette) {
    ////     Ok(dithered_image) => {
    ////         if let Err(e) = dithered_image.save(output_dither_bmp_path) {
    ////             log::error(&format!("Failed to save dithered indexed image: {}", e));
    ////             log::show_cursor();
    ////             return Err(format!("Failed to save dithered indexed image: {}", e));
    ////         }
    ////     }
    ////     Err(e) => {
    ////         log::error(&format!("Failed to convert indexed image: {}", e));
    ////         log::show_cursor();
    ////         return Err(format!("Failed to convert indexed image: {}", e));
    ////     }
    //// }
    ////
    //// log::ok(
    ////     "dithered indexed image saved successfully",
    ////     Some(benchmark::get_elapsed_time(local_timer)),
    //// );

    //
    // Parallel Stucki dithering
    //

    log::action("Dithering image using Stucki algorithm...");

    local_timer = benchmark::get_start_time();

    let indexed_dither_pixels = match dither::stucki(
        &original_rgba_img,
        width,
        height,
        &final_palette,
        parallel_strips_count,
    ) {
        Ok(results) => {
            log::ok(
                "image dithering completed successfully",
                Some(benchmark::get_elapsed_time(local_timer)),
            );
            results
        }
        Err(e) => {
            log::error(&format!("Error during dithering: {}", e));
            //// log::show_cursor();
            return Err(format!("Error during dithering: {}", e));
        }
    };

    //// //
    //// // Save the dithered indexed image as BMP
    //// //
    ////
    //// log::action("Saving dithered indexed image as BMP...");
    ////
    //// local_timer = benchmark::get_start_time();
    ////
    //// match image::from_indexed(&indexed_dither_pixels, width, height, &final_palette) {
    ////     Ok(dithered_image) => {
    ////         if let Err(e) = dithered_image.save(output_dither_bmp_path1) {
    ////             log::error(&format!("Failed to save dithered indexed image: {}", e));
    ////             log::show_cursor();
    ////             return Err(format!("Failed to save dithered indexed image: {}", e));
    ////         }
    ////     }
    ////     Err(e) => {
    ////         log::error(&format!("Failed to convert indexed image: {}", e));
    ////         log::show_cursor();
    ////         return Err(format!("Failed to convert indexed image: {}", e));
    ////     }
    //// }
    ////
    //// log::ok(
    ////     "dithered indexed image saved successfully",
    ////     Some(benchmark::get_elapsed_time(local_timer)),
    //// );

    //
    // Show total execution time
    //

    log::nl();
    let total_execution_time = benchmark::get_elapsed_time(app_timer);
    log::info(&format!("🮊 Done {:.2?}", total_execution_time));
    //// log::show_cursor();

    Ok((
		raw_palette,
		indexed_dither_pixels
	))
}
