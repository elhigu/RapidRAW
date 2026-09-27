//! JPEG encoding for on-screen previews.
//!
//! A large image is split into horizontal strips that are encoded on separate threads and joined
//! into one baseline JPEG, with a restart marker between strips. The strips start on 16-row
//! boundaries, the height of a 4:2:0 MCU, and every encoding step up to the entropy coder works
//! within an MCU row. A restart marker resets the DC predictors, which is the state each strip was
//! encoded from, so the joined file decodes to the same pixels as a single-pass encode.

use imgref::ImgRef;
use mozjpeg_rs::{Encoder, Preset};
use rayon::prelude::*;
use rgb::RGBA8;

const MCU_SIZE: usize = 16;
const MIN_STRIP_ROWS: usize = 128;

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOF0: u8 = 0xC0;
const DHT: u8 = 0xC4;
const DQT: u8 = 0xDB;
const DRI: u8 = 0xDD;
const SOS: u8 = 0xDA;
const RST0: u8 = 0xD0;

pub fn encode_rgba(
    image: ImgRef<RGBA8>,
    quality: u8,
    fast_color: bool,
) -> mozjpeg_rs::Result<Vec<u8>> {
    let strips = (image.height() / MIN_STRIP_ROWS).min(rayon::current_num_threads());
    encode_in_strips(image, quality, fast_color, strips)
}

fn encode_single(
    image: ImgRef<RGBA8>,
    quality: u8,
    fast_color: bool,
) -> mozjpeg_rs::Result<Vec<u8>> {
    Encoder::new(Preset::BaselineFastest)
        .quality(quality)
        .fast_color(fast_color)
        .encode_imgref(image)
}

fn encode_in_strips(
    image: ImgRef<RGBA8>,
    quality: u8,
    fast_color: bool,
    strips: usize,
) -> mozjpeg_rs::Result<Vec<u8>> {
    let (width, height) = (image.width(), image.height());
    if strips < 2 || width == 0 {
        return encode_single(image, quality, fast_color);
    }
    let strip_rows = height.div_ceil(strips).next_multiple_of(MCU_SIZE);
    let restart_interval = width.div_ceil(MCU_SIZE) * (strip_rows / MCU_SIZE);
    let Ok(restart_interval) = u16::try_from(restart_interval) else {
        return encode_single(image, quality, fast_color);
    };

    let tops: Vec<usize> = (0..height).step_by(strip_rows).collect();
    if tops.len() < 2 {
        return encode_single(image, quality, fast_color);
    }
    let parts = tops
        .into_par_iter()
        .map(|top| {
            let rows = strip_rows.min(height - top);
            encode_single(image.sub_image(0, top, width, rows), quality, fast_color)
        })
        .collect::<Result<Vec<_>, _>>()?;

    match join_strips(&parts, height, restart_interval) {
        Some(joined) => Ok(joined),
        None => encode_single(image, quality, fast_color),
    }
}

struct Parsed<'a> {
    /// Everything from SOI up to, but not including, the SOS marker.
    header: &'a [u8],
    sof_offset: usize,
    sos_segment: &'a [u8],
    entropy_data: &'a [u8],
    tables: Vec<&'a [u8]>,
}

fn parse(jpeg: &[u8]) -> Option<Parsed<'_>> {
    if jpeg.len() < 4 || jpeg[..2] != [0xFF, SOI] || jpeg[jpeg.len() - 2..] != [0xFF, EOI] {
        return None;
    }
    let mut pos = 2;
    let mut sof_offset = None;
    let mut tables = Vec::new();
    loop {
        if *jpeg.get(pos)? != 0xFF {
            return None;
        }
        let marker = *jpeg.get(pos + 1)?;
        let length = u16::from_be_bytes([*jpeg.get(pos + 2)?, *jpeg.get(pos + 3)?]) as usize;
        let segment = jpeg.get(pos..pos + 2 + length)?;
        match marker {
            SOF0 => sof_offset = Some(pos),
            DQT | DHT => tables.push(segment),
            DRI => return None,
            SOS => {
                return Some(Parsed {
                    header: &jpeg[..pos],
                    sof_offset: sof_offset?,
                    sos_segment: segment,
                    entropy_data: &jpeg[pos + 2 + length..jpeg.len() - 2],
                    tables,
                });
            }
            0xC1..=0xCF => return None,
            _ => {}
        }
        pos += 2 + length;
    }
}

fn join_strips(parts: &[Vec<u8>], height: usize, restart_interval: u16) -> Option<Vec<u8>> {
    let parsed = parts.iter().map(|p| parse(p)).collect::<Option<Vec<_>>>()?;
    let first = &parsed[0];
    // The strips must share their tables and differ only in the image height.
    let same_frame = |p: &Parsed| {
        p.tables == first.tables
            && p.sos_segment == first.sos_segment
            && p.header.len() == first.header.len()
            && p.sof_offset == first.sof_offset
            && p.header[..first.sof_offset + 5] == first.header[..first.sof_offset + 5]
            && p.header[first.sof_offset + 7..] == first.header[first.sof_offset + 7..]
    };
    if !parsed.iter().all(same_frame) {
        return None;
    }

    let entropy_len: usize = parsed.iter().map(|p| p.entropy_data.len() + 2).sum();
    let mut out =
        Vec::with_capacity(first.header.len() + 6 + first.sos_segment.len() + entropy_len);
    out.extend_from_slice(first.header);
    let height = u16::try_from(height).ok()?.to_be_bytes();
    out[first.sof_offset + 5..first.sof_offset + 7].copy_from_slice(&height);
    out.extend_from_slice(&[0xFF, DRI, 0, 4]);
    out.extend_from_slice(&restart_interval.to_be_bytes());
    out.extend_from_slice(first.sos_segment);
    for (i, part) in parsed.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&[0xFF, RST0 + ((i - 1) % 8) as u8]);
        }
        out.extend_from_slice(part.entropy_data);
    }
    out.extend_from_slice(&[0xFF, EOI]);
    Some(out)
}

#[cfg(test)]
mod tests {
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
                for strips in [2, 3, 7] {
                    let joined = encode_in_strips(image, quality, fast_color, strips).unwrap();
                    assert_ne!(single, joined, "{width}x{height} was not split");
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
}
