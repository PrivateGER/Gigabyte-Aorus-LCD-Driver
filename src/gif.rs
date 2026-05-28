use crate::image::{RgbImage, rgb565_le};
use crate::protocol::{LCD_HEIGHT, LCD_WIDTH, animation_container};
use std::fs;
use std::io;
use std::path::Path;

pub const MAX_GIF_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_GIF_PAYLOAD_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_GIF_FRAMES_EXCLUSIVE: usize = 80;
pub const MAX_GIF_CANVAS_PIXELS: u64 = LCD_WIDTH as u64 * LCD_HEIGHT as u64 * 16;
pub const MAX_GIF_FPS: u16 = 10;
pub const MAX_GIF_FRAMES_DEFAULT: usize = 24;
pub const MAX_GIF_CONTENT_HEIGHT_DEFAULT: u32 = 150;
pub const MIN_GIF_DELAY_MS: u16 = 1000 / MAX_GIF_FPS;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GifFrame {
    pub image: RgbImage,
    pub delay_ms: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedGif {
    pub frames: Vec<GifFrame>,
    pub payload: Vec<u8>,
    pub upload_delay_ms: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GifLimits {
    pub width: u32,
    pub height: u32,
    pub min_delay_ms: u16,
    pub max_frames_exclusive: usize,
    pub max_content_width: u32,
    pub max_content_height: u32,
    pub max_canvas_pixels: u64,
    pub max_input_bytes: usize,
    pub max_payload_bytes: usize,
}

impl Default for GifLimits {
    fn default() -> Self {
        Self {
            width: LCD_WIDTH,
            height: LCD_HEIGHT,
            min_delay_ms: MIN_GIF_DELAY_MS,
            max_frames_exclusive: MAX_GIF_FRAMES_DEFAULT + 1,
            max_content_width: LCD_WIDTH,
            max_content_height: MAX_GIF_CONTENT_HEIGHT_DEFAULT,
            max_canvas_pixels: MAX_GIF_CANVAS_PIXELS,
            max_input_bytes: MAX_GIF_INPUT_BYTES,
            max_payload_bytes: MAX_GIF_PAYLOAD_BYTES,
        }
    }
}

pub fn gif_payload_from_path(path: &Path, limits: &GifLimits) -> io::Result<NormalizedGif> {
    if let Ok(metadata) = fs::metadata(path)
        && metadata.is_file()
        && metadata.len() > limits.max_input_bytes as u64
    {
        return Err(gif_input_too_large(
            metadata.len(),
            limits.max_input_bytes as u64,
        ));
    }
    let bytes = fs::read(path)?;
    if bytes.len() > limits.max_input_bytes {
        return Err(gif_input_too_large(
            bytes.len() as u64,
            limits.max_input_bytes as u64,
        ));
    }
    let frames = decode_standard_gif(&bytes, limits)?;
    normalize_frames(frames, limits)
}

fn gif_input_too_large(actual: u64, limit: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("GIF input is too large: {actual} bytes > {limit} bytes"),
    )
}

pub fn normalize_frames(frames: Vec<GifFrame>, limits: &GifLimits) -> io::Result<NormalizedGif> {
    if frames.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "GIF must contain at least one frame",
        ));
    }

    let mut frames = optimize_timing(frames, limits)
        .into_iter()
        .map(|frame| GifFrame {
            image: letterbox_to_lcd(
                &frame.image,
                limits.width,
                limits.height,
                limits.max_content_width,
                limits.max_content_height,
            ),
            delay_ms: frame.delay_ms,
        })
        .collect::<Vec<_>>();

    loop {
        let payload = payload_for_frames(&frames, limits)?;
        if payload.len() <= limits.max_payload_bytes {
            let upload_delay_ms = frames
                .iter()
                .map(|frame| frame.delay_ms)
                .max()
                .unwrap_or(limits.min_delay_ms)
                .min(u8::MAX as u16) as u8;
            return Ok(NormalizedGif {
                frames,
                payload,
                upload_delay_ms,
            });
        }
        if frames.len() == 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "single-frame GIF payload is too large: {} bytes > {} bytes",
                    payload.len(),
                    limits.max_payload_bytes
                ),
            ));
        }
        let next_len = frames.len() - 1;
        frames = sample_frames(frames, next_len);
    }
}

fn payload_for_frames(frames: &[GifFrame], limits: &GifLimits) -> io::Result<Vec<u8>> {
    let rgb565_frames = frames
        .iter()
        .map(|frame| rgb565_le(&frame.image))
        .collect::<io::Result<Vec<_>>>()?;
    animation_container(&rgb565_frames, limits.width, limits.height)
}

fn optimize_timing(frames: Vec<GifFrame>, limits: &GifLimits) -> Vec<GifFrame> {
    let max_frames = limits.max_frames_exclusive.saturating_sub(1).max(1);
    let mut frames = coalesce_fast_frames(frames, limits.min_delay_ms);
    if frames.len() > max_frames {
        frames = sample_frames_preserving_duration(frames, max_frames);
    }
    frames
        .into_iter()
        .map(|mut frame| {
            frame.delay_ms = frame.delay_ms.max(limits.min_delay_ms);
            frame
        })
        .collect()
}

fn coalesce_fast_frames(frames: Vec<GifFrame>, min_delay_ms: u16) -> Vec<GifFrame> {
    let mut optimized = Vec::new();
    let mut pending_delay = 0u32;
    let mut pending_frame: Option<GifFrame> = None;

    for frame in frames {
        if pending_frame.is_none() {
            pending_frame = Some(GifFrame {
                image: frame.image,
                delay_ms: 0,
            });
        } else if pending_delay >= min_delay_ms as u32 {
            let mut ready = pending_frame
                .replace(GifFrame {
                    image: frame.image,
                    delay_ms: 0,
                })
                .unwrap();
            ready.delay_ms = pending_delay.min(u16::MAX as u32) as u16;
            optimized.push(ready);
            pending_delay = 0;
        }
        pending_delay += frame.delay_ms.max(1) as u32;
    }

    if let Some(mut frame) = pending_frame {
        frame.delay_ms = pending_delay.max(min_delay_ms as u32).min(u16::MAX as u32) as u16;
        optimized.push(frame);
    }

    optimized
}

fn sample_frames_preserving_duration(frames: Vec<GifFrame>, max_frames: usize) -> Vec<GifFrame> {
    let target_duration = frames
        .iter()
        .map(|frame| frame.delay_ms as u32)
        .sum::<u32>();
    let mut sampled = sample_frames(frames, max_frames);
    if target_duration == 0 {
        return sampled;
    }
    let sampled_duration = sampled
        .iter()
        .map(|frame| frame.delay_ms as u32)
        .sum::<u32>();
    if let Some(last) = sampled.last_mut() {
        last.delay_ms = (last.delay_ms as u32 + target_duration.saturating_sub(sampled_duration))
            .min(u16::MAX as u32) as u16;
    }
    sampled
}

fn sample_frames(mut frames: Vec<GifFrame>, max_frames: usize) -> Vec<GifFrame> {
    if frames.len() <= max_frames {
        return frames;
    }
    let source_len = frames.len();
    let mut sampled = Vec::with_capacity(max_frames);
    for output_index in 0..max_frames {
        let source_index = if max_frames == 1 {
            0
        } else {
            output_index * (source_len - 1) / (max_frames - 1)
        };
        sampled.push(frames[source_index].clone());
    }
    frames.clear();
    sampled
}

fn letterbox_to_lcd(
    source: &RgbImage,
    width: u32,
    height: u32,
    max_content_width: u32,
    max_content_height: u32,
) -> RgbImage {
    let mut canvas = RgbImage::blank(width, height, [0, 0, 0]);
    let content_width = max_content_width.min(width).max(1);
    let content_height = max_content_height.min(height).max(1);
    let scale = (content_width as f32 / source.width as f32)
        .min(content_height as f32 / source.height as f32)
        .min(1.0);
    let target_width = (source.width as f32 * scale).round().max(1.0) as u32;
    let target_height = (source.height as f32 * scale).round().max(1.0) as u32;
    let left = ((width - target_width) / 2) as i32;
    let top = ((height - target_height) / 2) as i32;

    for y in 0..target_height {
        for x in 0..target_width {
            let src_x = (x as u64 * source.width as u64 / target_width as u64) as u32;
            let src_y = (y as u64 * source.height as u64 / target_height as u64) as u32;
            canvas.set(
                (left + x as i32) as u32,
                (top + y as i32) as u32,
                source.get(src_x, src_y),
            );
        }
    }
    canvas
}

fn decode_standard_gif(bytes: &[u8], limits: &GifLimits) -> io::Result<Vec<GifFrame>> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut reader = options
        .read_info(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let canvas_width = reader.width() as u32;
    let canvas_height = reader.height() as u32;
    let canvas_pixels = u64::from(canvas_width)
        .checked_mul(u64::from(canvas_height))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "GIF logical screen dimensions overflow",
            )
        })?;
    if canvas_pixels == 0 || canvas_pixels > limits.max_canvas_pixels {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "GIF logical screen is too large: {canvas_width}x{canvas_height} = {canvas_pixels} pixels > {} pixels",
                limits.max_canvas_pixels
            ),
        ));
    }
    let mut canvas = vec![[0, 0, 0, 0]; (canvas_width * canvas_height) as usize];
    let mut frames = Vec::new();
    let source_frame_limit = limits.max_frames_exclusive.saturating_sub(1).max(1);
    while let Some(frame) = reader
        .read_next_frame()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
    {
        let previous_canvas = if frame.dispose == gif::DisposalMethod::Previous {
            Some(canvas.clone())
        } else {
            None
        };
        for y in 0..frame.height as u32 {
            for x in 0..frame.width as u32 {
                let src_index = ((y * frame.width as u32 + x) * 4) as usize;
                let pixel = &frame.buffer[src_index..src_index + 4];
                let alpha = pixel[3] as u16;
                if alpha == 0 {
                    continue;
                }
                let dst_x = frame.left as u32 + x;
                let dst_y = frame.top as u32 + y;
                if dst_x >= canvas_width || dst_y >= canvas_height {
                    continue;
                }
                canvas[(dst_y * canvas_width + dst_x) as usize] = [
                    ((pixel[0] as u16 * alpha + 127) / 255) as u8,
                    ((pixel[1] as u16 * alpha + 127) / 255) as u8,
                    ((pixel[2] as u16 * alpha + 127) / 255) as u8,
                    255,
                ];
            }
        }
        let pixels = canvas
            .iter()
            .map(|[red, green, blue, alpha]| {
                if *alpha == 0 {
                    [0, 0, 0]
                } else {
                    [*red, *green, *blue]
                }
            })
            .collect();
        let delay_ms = frame.delay.max(1).saturating_mul(10);
        frames.push(GifFrame {
            image: RgbImage::from_pixels(canvas_width, canvas_height, pixels)?,
            delay_ms,
        });
        if frames.len() > source_frame_limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "GIF contains too many source frames: {} > {}",
                    frames.len(),
                    source_frame_limit
                ),
            ));
        }
        match frame.dispose {
            gif::DisposalMethod::Background => {
                for y in 0..frame.height as u32 {
                    for x in 0..frame.width as u32 {
                        let dst_x = frame.left as u32 + x;
                        let dst_y = frame.top as u32 + y;
                        if dst_x < canvas_width && dst_y < canvas_height {
                            canvas[(dst_y * canvas_width + dst_x) as usize] = [0, 0, 0, 0];
                        }
                    }
                }
            }
            gif::DisposalMethod::Previous => {
                if let Some(previous_canvas) = previous_canvas {
                    canvas = previous_canvas;
                }
            }
            gif::DisposalMethod::Any | gif::DisposalMethod::Keep => {}
        }
    }
    Ok(frames)
}
