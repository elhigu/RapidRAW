use super::*;

fn test_image(width: usize, height: usize) -> Vec<RGBA8> {
    let mut state: u32 = 0x1234_5678;
    (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (state >> 27) as usize;
            RGBA8::new(
                ((x * 255 / width.max(1)) ^ noise) as u8,
                ((y * 255 / height.max(1)) + noise) as u8,
                ((x + y) * 3 + noise * 5) as u8,
                255,
            )
        })
        .collect()
}

fn decode(jpeg: &[u8]) -> Vec<u8> {
    image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .unwrap()
        .to_rgb8()
        .into_raw()
}

fn restart_markers(jpeg: &[u8]) -> Vec<u8> {
    let sos = jpeg.windows(2).position(|w| w == [0xFF, SOS]).unwrap();
    jpeg[sos..]
        .windows(2)
        .filter(|w| w[0] == 0xFF && (RST0..=RST0 + 7).contains(&w[1]))
        .map(|w| w[1])
        .collect()
}

#[test]
fn strips_decode_to_the_same_pixels_as_a_single_encode() {
    let sizes = [
        (1280, 853),
        (853, 1280),
        (37, 1000),
        (1000, 40),
        (1203, 777),
    ];
    for (width, height) in sizes {
        let pixels = test_image(width, height);
        let image = ImgRef::new(&pixels, width, height);
        for (quality, fast_color) in [(94, true), (92, false), (65, true)] {
            let single = encode_single(image, quality, fast_color).unwrap();
            for strips in [2, 3, 7, 12, 24] {
                let joined = encode_in_strips(image, quality, fast_color, strips).unwrap();
                assert_ne!(single, joined, "{width}x{height} was not split");
                let parts = height.div_ceil(strip_rows(width, height, strips));
                let expected: Vec<u8> = (0..parts - 1).map(|i| RST0 + (i % 8) as u8).collect();
                assert_eq!(
                    restart_markers(&joined),
                    expected,
                    "{width}x{height}, {strips} strips"
                );
                assert_eq!(
                    decode(&single),
                    decode(&joined),
                    "{width}x{height}, quality {quality}, {strips} strips"
                );
            }
        }
    }
}

#[test]
fn small_images_are_encoded_in_one_piece() {
    let pixels = test_image(64, 20);
    let image = ImgRef::new(&pixels, 64, 20);
    assert_eq!(
        encode_rgba(image, 90, true).unwrap(),
        encode_single(image, 90, true).unwrap()
    );
}

#[test]
fn strips_with_another_mcu_size_are_not_joined() {
    let (width, height) = (300, 256);
    let pixels = test_image(width, height);
    let image = ImgRef::new(&pixels, width, height);
    let parts: Vec<Vec<u8>> = [0, 128]
        .into_iter()
        .map(|top| {
            Encoder::new(Preset::BaselineFastest)
                .quality(90)
                .subsampling(mozjpeg_rs::Subsampling::S444)
                .encode_imgref(image.sub_image(0, top, width, 128))
                .unwrap()
        })
        .collect();
    let restart_interval = (width.div_ceil(8) * 16) as u16;
    assert!(join_strips(&parts, height, restart_interval).is_none());
}

#[test]
fn restart_intervals_of_large_images_fit_in_16_bits() {
    for (width, height, strips) in [
        (9504, 6336, 2),
        (11648, 8736, 4),
        (30000, 5000, 8),
        (65535, 300, 2),
        (6000, 4000, 24),
    ] {
        let rows = strip_rows(width, height, strips);
        assert!(
            rows > 0 && rows.is_multiple_of(MCU_SIZE),
            "{width}x{height}: {rows} rows"
        );
        assert!(
            width.div_ceil(MCU_SIZE) * rows / MCU_SIZE <= usize::from(u16::MAX),
            "{width}x{height}, {strips} strips"
        );
    }
    assert_eq!(strip_rows(6000, 4000, 24), 176);
}
