//! Turns an animated GIF, WebP or APNG into the H.264 MP4 that WhatsApp
//! plays as a GIF. WhatsApp shows a picture file as a still, so a moving
//! picture travels as a looping video without sound.

use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, Profile, RateControlMode};
use openh264::formats::{RgbSliceU8, YUVBuffer};

/// The longer side of the video, in pixels. Larger GIFs are scaled down.
const LONGEST_SIDE: u32 = 640;
/// Encoder bitrate: GIFs are short, flat pictures, so this is plenty.
const BITRATE: u32 = 1_500_000;
/// Media time in milliseconds, the unit frame delays come in.
const TIMESCALE: u32 = 1000;

/// The MP4 for an animated picture, or `None` when it does not move.
pub(crate) fn encode(bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let Some(frames) = super::sticker_maker::animation_frames(bytes) else {
        return Ok(None);
    };
    encode_frames(&frames).map(Some)
}

fn encode_frames(frames: &[(image::RgbaImage, u32)]) -> Result<Vec<u8>, String> {
    let (width, height) = frames
        .first()
        .map(|(frame, _)| video_size(frame.width(), frame.height()))
        .ok_or("The picture has no frames")?;
    let total: u32 = frames.iter().map(|(_, delay)| *delay).sum();
    let fps = frames.len() as f32 * 1000.0 / total.max(1) as f32;
    let config = EncoderConfig::new()
        .profile(Profile::Baseline)
        .bitrate(BitRate::from_bps(BITRATE))
        .rate_control_mode(RateControlMode::Bitrate)
        .max_frame_rate(FrameRate::from_hz(fps.clamp(1.0, 50.0)))
        .skip_frames(false);
    let mut encoder = Encoder::with_api_config(openh264::OpenH264API::from_source(), config)
        .map_err(|error| error.to_string())?;
    let mut samples = Vec::with_capacity(frames.len());
    let (mut sps, mut pps) = (None, None);
    for (frame, delay) in frames {
        let rgb = flatten(frame, width, height);
        let yuv =
            YUVBuffer::from_rgb8_source(RgbSliceU8::new(&rgb, (width as usize, height as usize)));
        let stream = encoder
            .encode(&yuv)
            .map_err(|error| error.to_string())?
            .to_vec();
        let mut sample = Vec::new();
        let mut sync = false;
        for nal in nal_units(&stream) {
            match nal[0] & 0x1F {
                7 => sps = Some(nal.to_vec()),
                8 => pps = Some(nal.to_vec()),
                kind => {
                    sync |= kind == 5;
                    sample.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    sample.extend_from_slice(nal);
                }
            }
        }
        samples.push((sample, (*delay).max(10), sync));
    }
    let (Some(sps), Some(pps)) = (sps, pps) else {
        return Err("The video encoder gave no stream header".to_owned());
    };
    let mut writer = mp4::Mp4Writer::write_start(
        std::io::Cursor::new(Vec::new()),
        &mp4::Mp4Config {
            major_brand: "isom".parse().map_err(|_| "bad brand")?,
            minor_version: 512,
            compatible_brands: ["isom", "iso2", "avc1", "mp41"]
                .iter()
                .filter_map(|brand| brand.parse().ok())
                .collect(),
            timescale: TIMESCALE,
        },
    )
    .map_err(|error| error.to_string())?;
    writer
        .add_track(&mp4::TrackConfig {
            track_type: mp4::TrackType::Video,
            timescale: TIMESCALE,
            language: "und".to_owned(),
            media_conf: mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
                width: width as u16,
                height: height as u16,
                seq_param_set: sps,
                pic_param_set: pps,
            }),
        })
        .map_err(|error| error.to_string())?;
    let mut start = 0u64;
    for (sample, duration, is_sync) in samples {
        writer
            .write_sample(
                1,
                &mp4::Mp4Sample {
                    start_time: start,
                    duration,
                    rendering_offset: 0,
                    is_sync,
                    bytes: mp4::Bytes::from(sample),
                },
            )
            .map_err(|error| error.to_string())?;
        start += u64::from(duration);
    }
    writer.write_end().map_err(|error| error.to_string())?;
    Ok(writer.into_writer().into_inner())
}

/// The video's size: the picture's, scaled down to [`LONGEST_SIDE`] and
/// rounded to even numbers, which 4:2:0 video needs.
fn video_size(width: u32, height: u32) -> (u32, u32) {
    let scale = (LONGEST_SIDE as f32 / width.max(height) as f32).min(1.0);
    let even = |side: u32| ((side as f32 * scale) as u32 & !1).max(2);
    (even(width), even(height))
}

/// A frame at the video's size as RGB, with see-through parts on white.
fn flatten(frame: &image::RgbaImage, width: u32, height: u32) -> Vec<u8> {
    let frame = if frame.dimensions() == (width, height) {
        std::borrow::Cow::Borrowed(frame)
    } else {
        std::borrow::Cow::Owned(image::imageops::resize(
            frame,
            width,
            height,
            image::imageops::FilterType::Triangle,
        ))
    };
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in frame.pixels() {
        let [red, green, blue, alpha] = pixel.0;
        let alpha = u16::from(alpha);
        for channel in [red, green, blue] {
            rgb.push(((u16::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8);
        }
    }
    rgb
}

/// The NAL units of an Annex B stream, without their start codes.
fn nal_units(stream: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut starts = Vec::new();
    let mut at = 0;
    while at + 3 <= stream.len() {
        if stream[at..at + 3] == [0, 0, 1] {
            starts.push(at + 3);
            at += 3;
        } else {
            at += 1;
        }
    }
    let ends: Vec<usize> = starts
        .iter()
        .skip(1)
        .map(|&next| {
            // A four-byte start code leaves a zero before the three-byte one.
            let end = next - 3;
            if end > 0 && stream[end - 1] == 0 {
                end - 1
            } else {
                end
            }
        })
        .chain(std::iter::once(stream.len()))
        .collect();
    starts
        .into_iter()
        .zip(ends)
        .map(move |(start, end)| &stream[start..end])
        .filter(|nal| !nal.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moving_gif() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for shade in [0u8, 120, 240] {
                let picture =
                    image::RgbaImage::from_pixel(101, 61, image::Rgba([shade, 40, 200, 255]));
                encoder
                    .encode_frame(image::Frame::from_parts(
                        picture,
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(80, 1),
                    ))
                    .unwrap();
            }
        }
        bytes
    }

    /// The MP4 holds every frame at an even size and lasts as long as the GIF.
    #[test]
    fn an_animated_gif_becomes_an_h264_video() {
        let video = encode(&moving_gif()).unwrap().expect("the GIF moves");
        let reader =
            mp4::Mp4Reader::read_header(std::io::Cursor::new(&video), video.len() as u64).unwrap();
        let track = reader.tracks().values().next().unwrap();
        assert_eq!(track.media_type().unwrap(), mp4::MediaType::H264);
        assert_eq!((track.width(), track.height()), (100, 60));
        assert_eq!(track.sample_count(), 3);
        assert_eq!(reader.duration().as_millis(), 240);
        let frames = crate::backend::sticker_maker::inspect(&video).unwrap();
        assert!(frames.animated, "the video decodes as moving");
    }

    #[test]
    fn a_still_picture_is_left_alone() {
        let mut png = Vec::new();
        image::RgbaImage::new(8, 8)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(encode(&png).unwrap(), None);
    }

    #[test]
    fn large_pictures_shrink_to_even_sides() {
        assert_eq!(video_size(1280, 721), (640, 360));
        assert_eq!(video_size(99, 51), (98, 50));
    }
}
