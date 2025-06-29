use image::{Rgba, RgbaImage};
use rand::prelude::IndexedRandom;
use rand::seq::SliceRandom;
use rand::rng;
use rayon::prelude::*;
use std::collections::HashMap;
use std::error::Error;

fn euclidean_distance(p1: &[f64; 4], p2: &[f64; 4]) -> f64 {
    (p1[0] - p2[0]).powi(2)
        + (p1[1] - p2[1]).powi(2)
        + (p1[2] - p2[2]).powi(2)
        + (p1[3] - p2[3]).powi(2)
}

pub fn multithreaded(
    image: &RgbaImage,
    k: usize,
    predefined_palette: &[Rgba<u8>],
) -> Result<(Vec<u8>, Vec<Rgba<u8>>), Box<dyn Error>> {
    if k == 0 {
        return Err("K (number of colors) must be greater than 0.".into());
    }
    if predefined_palette.len() > k {
        return Err("Number of predefined colors cannot exceed total K.".into());
    }

    let (width, height) = image.dimensions();
    let total_pixels = (width * height) as usize;

    if total_pixels == 0 {
        return Ok((Vec::new(), Vec::new()));
    }

    let data_points: Vec<[f64; 4]> = image
        .pixels()
        .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64, p[3] as f64])
        .collect();

    let mut centroids: Vec<[f64; 4]> = Vec::with_capacity(k);
    let mut rng1 = rng();

    let fixed_centroid_count = predefined_palette.len();
    for p_color in predefined_palette {
        centroids.push([
            p_color[0] as f64,
            p_color[1] as f64,
            p_color[2] as f64,
            p_color[3] as f64,
        ]);
    }

    let num_random_centroids = k - fixed_centroid_count;
    if num_random_centroids > 0 {
        let mut available_pixels_indices: Vec<usize> = (0..total_pixels).collect();
        available_pixels_indices.shuffle(&mut rng1);

        for i in 0..num_random_centroids {
            if let Some(&pixel_idx) = available_pixels_indices.get(i) {
                centroids.push(data_points[pixel_idx]);
            } else {
                eprintln!("Warning: Not enough unique pixels to initialize all random centroids.");
                break;
            }
        }
    }

    let max_iterations = 100;
    let tolerance = 0.01;

    let mut old_centroids_sum_sq_diff = f64::INFINITY;

    let mut assignments: Vec<usize> = vec![0; total_pixels];

    for iter in 0..max_iterations {
        assignments.par_iter_mut().enumerate().for_each(|(i, assigned_idx)| {
            let pixel = &data_points[i];
            let mut min_dist = f64::INFINITY;
            let mut closest_centroid_idx = 0;

            for (j, centroid) in centroids.iter().enumerate() {
                let dist = euclidean_distance(pixel, centroid);
                if dist < min_dist {
                    min_dist = dist;
                    closest_centroid_idx = j;
                }
            }
            *assigned_idx = closest_centroid_idx;
        });

        let aggregated_sums = assignments
            .par_iter()
            .zip(&data_points)
            .fold(
                || HashMap::new(),
                |mut acc: HashMap<usize, ([f64; 4], usize)>, (&cluster_idx, pixel_value)| {
                    let (sum, count) = acc.entry(cluster_idx).or_insert(([0.0; 4], 0));
                    sum[0] += pixel_value[0];
                    sum[1] += pixel_value[1];
                    sum[2] += pixel_value[2];
                    sum[3] += pixel_value[3];
                    *count += 1;
                    acc
                },
            )
            .reduce(
                || HashMap::new(),
                |mut acc1, acc2| {
                    for (cluster_idx, (sum2, count2)) in acc2 {
                        let (sum1, count1) = acc1.entry(cluster_idx).or_insert(([0.0; 4], 0));
                        sum1[0] += sum2[0];
                        sum1[1] += sum2[1];
                        sum1[2] += sum2[2];
                        sum1[3] += sum2[3];
                        *count1 += count2;
                    }
                    acc1
                },
            );

        let mut new_centroids: Vec<[f64; 4]> = vec![[0.0; 4]; k];
        let mut current_centroids_sum_sq_diff = 0.0;

        for i in 0..k {
            if i < fixed_centroid_count {
                new_centroids[i] = [
                    predefined_palette[i][0] as f64,
                    predefined_palette[i][1] as f64,
                    predefined_palette[i][2] as f64,
                    predefined_palette[i][3] as f64,
                ];
                current_centroids_sum_sq_diff += euclidean_distance(&centroids[i], &new_centroids[i]);
                continue;
            }

            if let Some(&(sum, count)) = aggregated_sums.get(&i) {
                if count > 0 {
                    new_centroids[i][0] = sum[0] / count as f64;
                    new_centroids[i][1] = sum[1] / count as f64;
                    new_centroids[i][2] = sum[2] / count as f64;
                    new_centroids[i][3] = sum[3] / count as f64;
                } else {
                    if let Some(&pixel_value) = data_points.as_slice().choose(&mut rng1) {
                        new_centroids[i] = pixel_value;
                    } else {
                        new_centroids[i] = centroids[i];
                    }
                }
            } else {
                if let Some(&pixel_value) = data_points.as_slice().choose(&mut rng1) {
                    new_centroids[i] = pixel_value;
                } else {
                    new_centroids[i] = centroids[i];
                }
            }
            current_centroids_sum_sq_diff += euclidean_distance(&centroids[i], &new_centroids[i]);
        }

        if (old_centroids_sum_sq_diff - current_centroids_sum_sq_diff).abs() < tolerance {
            println!("K-means converged after {} iterations.", iter + 1);
            break;
        }
        old_centroids_sum_sq_diff = current_centroids_sum_sq_diff;
        centroids = new_centroids;
    }

    let indexed_pixels: Vec<u8> = assignments
        .iter()
        .map(|&idx| idx as u8)
        .collect();

    let final_palette: Vec<Rgba<u8>> = centroids
        .into_iter()
        .map(|c| {
            Rgba([
                c[0].round().max(0.0).min(255.0) as u8,
                c[1].round().max(0.0).min(255.0) as u8,
                c[2].round().max(0.0).min(255.0) as u8,
                c[3].round().max(0.0).min(255.0) as u8,
            ])
        })
        .collect();

    Ok((indexed_pixels, final_palette))
}
