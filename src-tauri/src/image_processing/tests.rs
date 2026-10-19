use super::*;
use crate::test_support::{assert_bits_eq, sample_values};
use image::{ImageBuffer, Luma, Rgb, Rgba};

const W: u32 = 41;
const H: u32 = 19;

fn rgba32f_image() -> DynamicImage {
    let values = sample_values((W * H * 4) as usize);
    DynamicImage::ImageRgba32F(ImageBuffer::from_raw(W, H, values).unwrap())
}

#[test]
fn to_rgb32f_parallel_matches_image_crate_for_rgba_f32() {
    let image = rgba32f_image();
    assert_bits_eq(
        image.to_rgb32f().as_raw(),
        to_rgb32f_parallel(&image).as_raw(),
    );
}

#[test]
fn enhance_gives_the_same_result_for_rgba_and_rgb_input() {
    let values: Vec<f32> = sample_values((W * H * 4) as usize)
        .into_iter()
        .map(|v| {
            if v.is_finite() {
                v.clamp(0.0, 4.0)
            } else {
                0.5
            }
        })
        .collect();
    let rgba = DynamicImage::ImageRgba32F(ImageBuffer::from_raw(W, H, values).unwrap());
    let rgb = DynamicImage::ImageRgb32F(rgba.to_rgb32f());
    for (color_nr, sharpening) in [(0.0, 0.0), (2.0, 0.0), (0.0, 0.35), (2.0, 0.35)] {
        let mut from_rgba = rgba.clone();
        let mut from_rgb = rgb.clone();
        remove_raw_artifacts_and_enhance(&mut from_rgba, color_nr, sharpening);
        remove_raw_artifacts_and_enhance(&mut from_rgb, color_nr, sharpening);
        assert_bits_eq(
            from_rgb.as_rgb32f().unwrap().as_raw(),
            from_rgba.as_rgb32f().unwrap().as_raw(),
        );
    }
}

/// Upstream's scalar implementation, kept to check that the vectorized one gives identical
/// output.
mod reference {
    use super::super::{rgb_to_yc_only, yc_to_rgb};
    use image::DynamicImage;
    use rayon::prelude::*;

    pub fn remove_raw_artifacts_and_enhance(
        image: &mut DynamicImage,
        color_nr_inv_sigma: f32,
        sharpening_amount: f32,
    ) {
        let mut buffer = image.to_rgb32f();
        let w = buffer.width() as usize;
        let h = buffer.height() as usize;

        let mut ycbcr_buffer = vec![0.0f32; w * h * 3];

        let src = buffer.as_raw();

        ycbcr_buffer
            .par_chunks_mut(3)
            .zip(src.par_chunks(3))
            .for_each(|(dest, pixel)| {
                let (y, cb, cr) = rgb_to_yc_only(pixel[0], pixel[1], pixel[2]);
                dest[0] = y;
                dest[1] = cb;
                dest[2] = cr;
            });

        if color_nr_inv_sigma > 0.0 {
            let base_inv_sigma = color_nr_inv_sigma;
            const OFFSETS: [isize; 3] = [-5, -1, 3];
            const OFFSET_SQUARES: [f32; 3] = [25.0, 1.0, 9.0];

            buffer
                .par_chunks_mut(w * 3)
                .enumerate()
                .for_each(|(y, row)| {
                    let row_offset = y * w;
                    let h_isize = h as isize;
                    let w_isize = w as isize;
                    let y_isize = y as isize;

                    for x in 0..w {
                        let center_idx = (row_offset + x) * 3;

                        let cy = ycbcr_buffer[center_idx];
                        let ccb = ycbcr_buffer[center_idx + 1];
                        let ccr = ycbcr_buffer[center_idx + 2];

                        let mut cb_sum = 0.0;
                        let mut cr_sum = 0.0;
                        let mut w_sum = 0.0;

                        for (ki, &ky) in OFFSETS.iter().enumerate() {
                            let sy = y_isize + ky;
                            if sy < 0 || sy >= h_isize {
                                continue;
                            }

                            let neighbor_row_idx = (sy as usize) * w;
                            let ky_sq_div_50 = OFFSET_SQUARES[ki] * 0.02;

                            for (kj, &kx) in OFFSETS.iter().enumerate() {
                                let sx = (x as isize) + kx;
                                if sx < 0 || sx >= w_isize {
                                    continue;
                                }

                                let neighbor_idx = (neighbor_row_idx + sx as usize) * 3;

                                let neighbor_y = ycbcr_buffer[neighbor_idx];
                                let y_diff = (cy - neighbor_y).abs();

                                let val = y_diff * base_inv_sigma;
                                let spatial_penalty = OFFSET_SQUARES[kj] * 0.02 + ky_sq_div_50;

                                let weight = 1.0 / (1.0 + val * val + spatial_penalty);

                                cb_sum += ycbcr_buffer[neighbor_idx + 1] * weight;
                                cr_sum += ycbcr_buffer[neighbor_idx + 2] * weight;
                                w_sum += weight;
                            }
                        }

                        let (out_cb, out_cr) = if w_sum > 1e-4 {
                            let inv_w_sum = 1.0 / w_sum;
                            let filtered_cb = cb_sum * inv_w_sum;
                            let filtered_cr = cr_sum * inv_w_sum;

                            let orig_mag_sq = ccb * ccb + ccr * ccr;
                            let filt_mag_sq = filtered_cb * filtered_cb + filtered_cr * filtered_cr;

                            if filt_mag_sq > orig_mag_sq && orig_mag_sq > 1e-12 {
                                let scale = (orig_mag_sq / filt_mag_sq).sqrt();
                                (filtered_cb * scale, filtered_cr * scale)
                            } else {
                                (filtered_cb, filtered_cr)
                            }
                        } else {
                            (ccb, ccr)
                        };

                        let (r, g, b) = yc_to_rgb(cy, out_cb, out_cr);

                        let o = x * 3;
                        row[o] = r.max(0.0);
                        row[o + 1] = g.max(0.0);
                        row[o + 2] = b.max(0.0);
                    }
                });
        }

        if sharpening_amount > 0.0 {
            apply_gentle_detail_enhance(&mut buffer, &ycbcr_buffer, sharpening_amount);
        }

        *image = DynamicImage::ImageRgb32F(buffer);
    }

    fn apply_gentle_detail_enhance(
        buffer: &mut image::ImageBuffer<image::Rgb<f32>, Vec<f32>>,
        ycbcr_source: &[f32],
        amount: f32,
    ) {
        let w = buffer.width() as usize;
        let h = buffer.height() as usize;

        let mut temp_blur = vec![0.0; w * h];
        let radius = 2i32;

        temp_blur
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row)| {
                let row_offset = y * w;
                for (x, row_val) in row.iter_mut().enumerate() {
                    let mut sum = 0.0;
                    let mut count = 0;
                    for kx in -radius..=radius {
                        let sx = (x as i32 + kx).clamp(0, (w as i32) - 1) as usize;
                        sum += ycbcr_source[(row_offset + sx) * 3];
                        count += 1;
                    }
                    *row_val = sum / count as f32;
                }
            });

        let output = buffer.as_mut();

        output
            .par_chunks_mut(w * 3)
            .enumerate()
            .for_each(|(y, rgb_row)| {
                for x in 0..w {
                    let mut blur_sum = 0.0;
                    let mut count = 0;
                    for ky in -radius..=radius {
                        let sy = (y as i32 + ky).clamp(0, (h as i32) - 1) as usize;
                        blur_sum += temp_blur[sy * w + x];
                        count += 1;
                    }
                    let blurred_val = blur_sum / count as f32;

                    let original_luma = ycbcr_source[(y * w + x) * 3];

                    let detail = original_luma - blurred_val;

                    let edge_strength = detail.abs();
                    let adaptive_amount = if edge_strength > 0.1 {
                        amount * 0.3
                    } else {
                        amount
                    };
                    let boost = detail * adaptive_amount;

                    let r_idx = x * 3;
                    let g_idx = r_idx + 1;
                    let b_idx = r_idx + 2;

                    let r = rgb_row[r_idx];
                    let g = rgb_row[g_idx];
                    let b = rgb_row[b_idx];

                    let new_r = r + boost;
                    let new_g = g + boost;
                    let new_b = b + boost;

                    let max_val = new_r.max(new_g).max(new_b);
                    let min_val = new_r.min(new_g).min(new_b);

                    let scale = if max_val > 1.0 || min_val < 0.0 {
                        if max_val > 1.0 && min_val < 0.0 {
                            0.0
                        } else if max_val > 1.0 {
                            (1.0 - r.max(g).max(b)) / boost.max(0.001)
                        } else {
                            r.min(g).min(b) / (-boost).max(0.001)
                        }
                    } else {
                        1.0
                    };

                    let safe_boost = boost * scale.clamp(0.0, 1.0);

                    rgb_row[r_idx] = (r + safe_boost).max(0.0);
                    rgb_row[g_idx] = (g + safe_boost).max(0.0);
                    rgb_row[b_idx] = (b + safe_boost).max(0.0);
                }
            });
    }
}

fn enhance_test_images(w: u32, h: u32) -> [DynamicImage; 2] {
    let n = (w * h * 3) as usize;
    let smooth = ImageBuffer::from_fn(w, h, |x, y| {
        let base = 0.45 + 0.4 * ((x as f32) * 0.37).sin() * ((y as f32) * 0.23).cos();
        Rgb([base, base * 0.9 + 0.05, (1.05 - base).max(0.0)])
    });
    let noise: Vec<f32> = sample_values(n)
        .into_iter()
        .zip(smooth.as_raw())
        .map(|(v, &s)| if v.is_finite() { s + v * 0.02 } else { s })
        .collect();
    [
        DynamicImage::ImageRgb32F(ImageBuffer::from_raw(w, h, noise).unwrap()),
        DynamicImage::ImageRgb32F(ImageBuffer::from_raw(w, h, sample_values(n)).unwrap()),
    ]
}

#[test]
fn enhance_matches_the_scalar_reference() {
    let sizes = [
        (1, 1),
        (4, 3),
        (9, 9),
        (16, 12),
        (17, 10),
        (41, 19),
        (70, 23),
    ];
    let settings = [(0.0, 0.35), (2.0, 0.0), (14.0, 0.35), (1190.0, 1.0)];
    for (w, h) in sizes {
        for image in enhance_test_images(w, h) {
            for (color_nr, sharpening) in settings {
                let mut expected = image.clone();
                let mut actual = image.clone();
                reference::remove_raw_artifacts_and_enhance(&mut expected, color_nr, sharpening);
                remove_raw_artifacts_and_enhance(&mut actual, color_nr, sharpening);
                assert_bits_eq(
                    expected.as_rgb32f().unwrap().as_raw(),
                    actual.as_rgb32f().unwrap().as_raw(),
                );
            }
        }
    }
}

#[test]
fn enhance_keeps_empty_images_empty() {
    for (w, h) in [(0, 4), (3, 0)] {
        let mut image = DynamicImage::ImageRgba32F(ImageBuffer::new(w, h));
        remove_raw_artifacts_and_enhance(&mut image, 14.0, 0.35);
        assert_eq!(image.dimensions(), (w, h));
        assert!(image.as_rgb32f().is_some());
    }
}

#[test]
fn to_rgb32f_parallel_matches_image_crate_for_other_formats() {
    let images = [
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(W, H, |x, y| {
            Rgb([(x * 7) as u8, (y * 11) as u8, (x + y) as u8])
        })),
        DynamicImage::ImageRgba16(ImageBuffer::from_fn(W, H, |x, y| {
            Rgba([(x * 997) as u16, (y * 1553) as u16, 40000, (x * y) as u16])
        })),
        DynamicImage::ImageLuma8(ImageBuffer::from_fn(W, H, |x, y| Luma([(x ^ y) as u8]))),
    ];
    for image in images {
        assert_bits_eq(
            image.to_rgb32f().as_raw(),
            to_rgb32f_parallel(&image).as_raw(),
        );
    }
}
