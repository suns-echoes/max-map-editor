use crossbeam::thread;
use image::{Rgba, RgbaImage};
use rand::rng;
use rand::seq::SliceRandom;
use rayon::prelude::*;
use std::collections::HashMap;
use std::error::Error;
use std::f32;

use crate::image_to_wrl::log;

// --- Helper Functions ---

/// Converts Rgba<u8> to a Vec<f32> for K-means
fn rgba_to_f32_vec(color: Rgba<u8>) -> Vec<f32> {
    vec![
        color[0] as f32,
        color[1] as f32,
        color[2] as f32,
        color[3] as f32,
    ]
}

/// Converts Vec<f32> back to Rgba<u8>
fn f32_vec_to_rgba(vec: &[f32]) -> Rgba<u8> {
    Rgba([vec[0] as u8, vec[1] as u8, vec[2] as u8, vec[3] as u8])
}

/// Calculates Euclidean distance between two color vectors
fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// Multithreaded K-means color quantization with template palette
/// This function performs K-means clustering on the colors in the image,
/// using a template palette for the centroids.
/// It fills dynamic slots (0x00000000) with random unique colors from the image,
/// ensuring that the final palette contains a mix of predefined and dynamically generated colors.
pub fn multithreaded(
    image: &RgbaImage,
    k: usize,
    template_palette: &[Rgba<u8>],
    max_iterations: usize,
    tolerance: f32,
) -> Result<Vec<Rgba<u8>>, Box<dyn Error>> {
    if k == 0 {
        return Err("k must be greater than 0".into());
    }

    let mut current_progress: usize;
    let total_progress = 100;

    log::start_progress();

    //
    // Extract unique colors from the image
    //

    let mut unique_colors: HashMap<Rgba<u8>, usize> = HashMap::new();
    let num_threads = num_cpus::get().max(1);
    let height = image.height() as usize;
    let stripe_height = (height + num_threads - 1) / num_threads;

    let stripes: Vec<_> = (0..num_threads)
        .map(|i| {
            let y_start = i * stripe_height;
            let y_end = ((i + 1) * stripe_height).min(height);
            (y_start, y_end)
        })
        .filter(|(y_start, y_end)| y_start < y_end)
        .collect();

    let results = thread::scope(|s| {
        let mut handles = Vec::new();
        for (y_start, y_end) in stripes {
            let image = image;
            handles.push(s.spawn(move |_| {
                let mut local_map: HashMap<Rgba<u8>, usize> = HashMap::new();
                for y in y_start..y_end {
                    for x in 0..image.width() {
                        let pixel = image.get_pixel(x, y as u32);
                        *local_map.entry(*pixel).or_insert(0) += 1;
                    }
                }
                local_map
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    })
    .unwrap();

    for local_map in results {
        for (color, count) in local_map {
            *unique_colors.entry(color).or_insert(0) += count;
        }
    }

    current_progress = 50;
    log::progress(current_progress, total_progress);

    //
    // Centroid filling and K-means clustering
    //

    // Prepare data points for K-means: convert unique colors to Vec<f32> and count occurrences

    let data_points: Vec<(Vec<f32>, usize)> = unique_colors
        .into_iter()
        .map(|(color, count)| (rgba_to_f32_vec(color), count))
        .collect();

    // Prepare initial centroids based on the template palette

    let mut transparent_indices = Vec::new();
    let mut dynamic_indices = Vec::new();
    for (i, color) in template_palette.iter().enumerate() {
        if color[3] == 0 && (color[0] != 0 || color[1] != 0 || color[2] != 0) {
            transparent_indices.push(i);
        } else if color[0] == 0 && color[1] == 0 && color[2] == 0 && color[3] == 0 {
            dynamic_indices.push(i);
        }
    }

    // Initialize centroids: fixed slots from template_palette, dynamic slots as None

    let centroids: Vec<Option<Vec<f32>>> = template_palette
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            if dynamic_indices.contains(&i) {
                None // dynamic slot
            } else {
                Some(rgba_to_f32_vec(c)) // fixed slot (non-transparent or transparent)
            }
        })
        .collect();

    // Fill dynamic slots with random unique colors from the image

    let mut rng1 = rng();
    let mut used_colors: Vec<Vec<f32>> = centroids.iter().filter_map(|c| c.clone()).collect();
    let mut available_data_points: Vec<_> = data_points
        .iter()
        .filter(|(dp_vec, _)| !used_colors.contains(dp_vec))
        .collect();
    available_data_points.shuffle(&mut rng1);

    current_progress = 50;
    log::progress(current_progress, total_progress);

    // Use the actual number of dynamic slots

    let dynamic_count = dynamic_indices.len();
    let mut dynamic_centroids: Vec<Vec<f32>> = Vec::new();
    for _ in 0..dynamic_count {
        if let Some((dp_vec, _)) = available_data_points.pop() {
            dynamic_centroids.push(dp_vec.clone());
            used_colors.push(dp_vec.clone());
        } else {
            dynamic_centroids.push(vec![0.0, 0.0, 0.0, 0.0]);
        }
    }

    // K-means on dynamic centroids only

    let mut dyn_centroids = dynamic_centroids.clone();
    let mut performed_iterations = 0;
    for iteration in 0..max_iterations {
        performed_iterations += 1;

        current_progress = 50 + (49 * iteration / max_iterations);
        log::progress(current_progress, total_progress);

        let assignments: Vec<usize> = data_points
            .par_iter()
            .map(|(data_point, _count)| {
                let mut min_dist = f32::MAX;
                let mut closest_centroid_idx = 0;
                for (i, centroid) in dyn_centroids.iter().enumerate() {
                    let dist = euclidean_distance(data_point, centroid);
                    if dist < min_dist {
                        min_dist = dist;
                        closest_centroid_idx = i;
                    }
                }
                closest_centroid_idx
            })
            .collect();

        // Group data points by cluster

        let mut clusters: Vec<Vec<(Vec<f32>, usize)>> =
            (0..dyn_centroids.len()).map(|_| Vec::new()).collect();
        for (i, assignment_idx) in assignments.into_iter().enumerate() {
            clusters[assignment_idx].push(data_points[i].clone());
        }

        let mut changed_centroids = false;
        let mut new_centroids = dyn_centroids.clone();

        for i in 0..dyn_centroids.len() {
            if clusters[i].is_empty() {
                continue;
            }
            let mut sum = vec![0.0; 4];
            let mut total_weight = 0.0;
            for (color_vec, count) in &clusters[i] {
                for j in 0..4 {
                    sum[j] += color_vec[j] * (*count as f32);
                }
                total_weight += *count as f32;
            }
            let new_centroid = sum.iter().map(|v| v / total_weight).collect::<Vec<f32>>();
            if euclidean_distance(&new_centroid, &dyn_centroids[i]) > tolerance {
                changed_centroids = true;
            }
            new_centroids[i] = new_centroid;
        }

        dyn_centroids = new_centroids;
        if !changed_centroids {
            break;
        }
    }

    //
    // Combine fixed and dynamic centroids into the final palette
    //

    let mut final_palette = template_palette.to_vec();
    let mut dyn_idx = 0;
    for &i in &dynamic_indices {
        if dyn_idx < dyn_centroids.len() {
            final_palette[i] = f32_vec_to_rgba(&dyn_centroids[dyn_idx]);
            dyn_idx += 1;
        } else {
            final_palette[i] = Rgba([0, 0, 0, 0]);
        }
    }

    log::line_up();
    log::info(&format!(
        "K-means completed in {} iterations, final palette size: {}",
        performed_iterations,
        final_palette.len()
    ));

    Ok(final_palette)
}
