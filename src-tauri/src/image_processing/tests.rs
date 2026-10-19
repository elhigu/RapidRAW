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
