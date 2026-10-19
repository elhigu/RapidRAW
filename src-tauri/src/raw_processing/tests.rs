use super::*;
use crate::test_support::{assert_bits_eq, sample_values};
use image::Rgb;

const W: u32 = 37;
const H: u32 = 23;

#[test]
fn rgb_pixels_to_image_keeps_every_value() {
    let values = sample_values((W * H * 3) as usize);
    let data: Vec<[f32; 3]> = values.as_chunks::<3>().0.to_vec();
    let expected =
        ImageBuffer::<Rgb<f32>, _>::from_fn(W, H, |x, y| Rgb(data[(y * W + x) as usize]));

    let actual = rgb_pixels_to_image(data, W, H).unwrap();

    assert_bits_eq(expected.as_raw(), actual.as_raw());
}

#[test]
fn mono_pixels_to_rgb_matches_serial_conversion() {
    let data = sample_values((W * H) as usize);
    let expected = ImageBuffer::<Rgb<f32>, _>::from_fn(W, H, |x, y| {
        let p = data[(y * W + x) as usize];
        Rgb([p, p, p])
    });

    let actual = mono_pixels_to_rgb(&data, W, H).unwrap();

    assert_bits_eq(expected.as_raw(), actual.as_raw());
}

#[test]
fn pixel_conversion_rejects_mismatched_dimensions() {
    let data = vec![[0.0f32; 3]; (W * H) as usize];
    assert!(rgb_pixels_to_image(data, W + 1, H).is_err());
    assert!(mono_pixels_to_rgb(&vec![0.0; (W * H) as usize], W, H + 1).is_err());
}
