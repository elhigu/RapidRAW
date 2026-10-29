use super::*;
use image::{ImageBuffer, Rgb};

fn values(n: usize) -> Vec<f32> {
    let special = [0.0, -0.0, 1.0, f32::NAN, f32::INFINITY, -2.5, 65504.0];
    let mut state: u32 = 0x9E37_79B9;
    (0..n)
        .map(|i| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if i % 11 == 0 {
                special[(i / 11) % special.len()]
            } else {
                (state >> 8) as f32 / (1u32 << 24) as f32 * 3.0 - 1.0
            }
        })
        .collect()
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

fn images(width: u32, height: u32) -> Vec<DynamicImage> {
    let n = (width * height) as usize;
    vec![
        DynamicImage::ImageRgb32F(ImageBuffer::from_raw(width, height, values(n * 3)).unwrap()),
        DynamicImage::ImageRgba32F(ImageBuffer::from_raw(width, height, values(n * 4)).unwrap()),
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([(x * 7) as u8, (y * 13) as u8, (x ^ y) as u8])
        })),
    ]
}

#[test]
fn rgba_conversion_matches_image_crate() {
    for image in images(23, 11) {
        assert_eq!(
            bits(to_rgba32f_parallel(&image).as_raw()),
            bits(image.to_rgba32f().as_raw())
        );
    }
}

#[test]
fn crop_matches_image_crate() {
    let rects = [
        (0, 0, 23, 11),
        (3, 2, 10, 5),
        (22, 10, 1, 1),
        (0, 4, 23, 7),
        (5, 0, 18, 11),
    ];
    for image in images(23, 11) {
        for (x, y, w, h) in rects {
            let expected = image.crop_imm(x, y, w, h);
            let actual = crop_image(&image, x, y, w, h);
            assert_eq!(expected.color(), actual.color());
            assert_eq!(expected.dimensions(), actual.dimensions());
            assert_eq!(expected.as_bytes(), actual.as_bytes(), "{x},{y} {w}x{h}");
        }
    }
}

#[test]
fn rgba_f32_images_are_borrowed_for_rotation() {
    let images = images(23, 11);
    assert!(matches!(to_rgba32f_parallel(&images[1]), Cow::Borrowed(_)));
    assert!(matches!(to_rgba32f_parallel(&images[0]), Cow::Owned(_)));
}
