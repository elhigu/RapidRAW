use super::*;
use crate::test_support::sample_values;
use image::{ImageBuffer, Luma, Rgb};

const W: u32 = 29;
const H: u32 = 31;

fn serial_reference(img: &DynamicImage) -> Vec<f16> {
    img.to_rgba32f()
        .into_raw()
        .into_iter()
        .map(f16::from_f32)
        .collect()
}

fn assert_same_bits(img: &DynamicImage) {
    let expected: Vec<u16> = serial_reference(img).iter().map(|v| v.to_bits()).collect();
    let actual: Vec<u16> = to_rgba_f16(img).iter().map(|v| v.to_bits()).collect();
    assert_eq!(expected, actual);
}

#[test]
fn to_rgba_f16_matches_serial_conversion_for_rgba_f32() {
    let values = sample_values((W * H * 4) as usize);
    assert_same_bits(&DynamicImage::ImageRgba32F(
        ImageBuffer::from_raw(W, H, values).unwrap(),
    ));
}

#[test]
fn to_rgba_f16_matches_serial_conversion_for_rgb_f32() {
    let values = sample_values((W * H * 3) as usize);
    assert_same_bits(&DynamicImage::ImageRgb32F(
        ImageBuffer::from_raw(W, H, values).unwrap(),
    ));
}

#[test]
fn to_rgba_f16_matches_serial_conversion_for_integer_formats() {
    assert_same_bits(&DynamicImage::ImageRgb8(ImageBuffer::from_fn(
        W,
        H,
        |x, y| Rgb([(x * 5) as u8, (y * 9) as u8, 200]),
    )));
    assert_same_bits(&DynamicImage::ImageLuma16(ImageBuffer::from_fn(
        W,
        H,
        |x, y| Luma([(x * y * 71) as u16]),
    )));
}
