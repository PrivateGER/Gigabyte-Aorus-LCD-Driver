use crate::protocol::{LCD_HEIGHT, LCD_WIDTH, single_frame_container};
use png::{BitDepth, ColorType, Decoder, Transformations};
use std::io::{self, Cursor};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 3]>,
}

impl RgbImage {
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<[u8; 3]>) -> io::Result<Self> {
        if pixels.len() != width as usize * height as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pixel count does not match image dimensions",
            ));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn blank(width: u32, height: u32, color: [u8; 3]) -> Self {
        Self {
            width,
            height,
            pixels: vec![color; width as usize * height as usize],
        }
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 3] {
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, pixel: [u8; 3]) {
        self.pixels[(y * self.width + x) as usize] = pixel;
    }
}

#[derive(Clone, Debug)]
struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<[u8; 4]>,
}

pub fn load_png_file(path: &Path) -> io::Result<RgbImage> {
    let bytes = std::fs::read(path)?;
    decode_png_rgb(&bytes)
}

pub fn decode_png_rgb(bytes: &[u8]) -> io::Result<RgbImage> {
    let rgba = decode_png_rgba(bytes)?;
    RgbImage::from_pixels(
        rgba.width,
        rgba.height,
        rgba.pixels
            .into_iter()
            .map(|[red, green, blue, alpha]| alpha_composite([red, green, blue], alpha, [0, 0, 0]))
            .collect(),
    )
}

pub fn mascot_background(png_bytes: &[u8]) -> io::Result<RgbImage> {
    let mascot = decode_png_rgba(png_bytes)?;
    Ok(render_mascot_background(&mascot))
}

pub fn mascot_background_from_path(path: &Path) -> io::Result<RgbImage> {
    let bytes = std::fs::read(path)?;
    mascot_background(&bytes)
}

fn decode_png_rgba(bytes: &[u8]) -> io::Result<RgbaImage> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(png_error)?;
    let buffer_size = reader.output_buffer_size().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "PNG output buffer would be too large",
        )
    })?;
    let mut buffer = vec![0; buffer_size];
    let info = reader.next_frame(&mut buffer).map_err(png_error)?;
    if info.bit_depth != BitDepth::Eight {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported PNG bit depth after normalization: {:?}",
                info.bit_depth
            ),
        ));
    }
    let data = &buffer[..info.buffer_size()];
    let pixels = match info.color_type {
        ColorType::Rgb => data
            .chunks_exact(3)
            .map(|px| [px[0], px[1], px[2], 255])
            .collect(),
        ColorType::Rgba => data
            .chunks_exact(4)
            .map(|px| [px[0], px[1], px[2], px[3]])
            .collect(),
        ColorType::Grayscale => data.iter().map(|v| [*v, *v, *v, 255]).collect(),
        ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .map(|px| [px[0], px[0], px[0], px[1]])
            .collect(),
        ColorType::Indexed => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "indexed PNG was not expanded by decoder",
            ));
        }
    };
    Ok(RgbaImage {
        width: info.width,
        height: info.height,
        pixels,
    })
}

fn png_error(error: png::DecodingError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn alpha_composite(source: [u8; 3], alpha: u8, background: [u8; 3]) -> [u8; 3] {
    let alpha = alpha as u16;
    let inverse = 255 - alpha;
    [
        ((source[0] as u16 * alpha + background[0] as u16 * inverse + 127) / 255) as u8,
        ((source[1] as u16 * alpha + background[1] as u16 * inverse + 127) / 255) as u8,
        ((source[2] as u16 * alpha + background[2] as u16 * inverse + 127) / 255) as u8,
    ]
}

fn render_mascot_background(source: &RgbaImage) -> RgbImage {
    let mut canvas = RgbImage::blank(LCD_WIDTH, LCD_HEIGHT, [0, 0, 0]);
    let scale = (170.0 / source.width as f32)
        .min(170.0 / source.height as f32)
        .min(1.0);
    let target_width = (source.width as f32 * scale).round().max(1.0) as u32;
    let target_height = (source.height as f32 * scale).round().max(1.0) as u32;
    let left = -8i32;
    let top = (LCD_HEIGHT as i32 - target_height as i32) / 2;

    for y in 0..target_height {
        for x in 0..target_width {
            let src_x = (x as u64 * source.width as u64 / target_width as u64) as u32;
            let src_y = (y as u64 * source.height as u64 / target_height as u64) as u32;
            let [red, green, blue, alpha] = source.pixels[(src_y * source.width + src_x) as usize];
            let dst_x = left + x as i32;
            let dst_y = top + y as i32;
            if dst_x < 0 || dst_y < 0 || dst_x >= LCD_WIDTH as i32 || dst_y >= LCD_HEIGHT as i32 {
                continue;
            }
            let existing = canvas.get(dst_x as u32, dst_y as u32);
            canvas.set(
                dst_x as u32,
                dst_y as u32,
                alpha_composite([red, green, blue], alpha, existing),
            );
        }
    }
    canvas
}

pub fn rgb565_le(image: &RgbImage) -> io::Result<Vec<u8>> {
    let mut output = Vec::with_capacity(image.pixels.len() * 2);
    for [red, green, blue] in image.pixels.iter().copied() {
        let value = ((red as u16 & 0xf8) << 8) | ((green as u16 & 0xfc) << 3) | (blue as u16 >> 3);
        output.extend_from_slice(&value.to_le_bytes());
    }
    Ok(output)
}

pub fn single_frame_payload(image: &RgbImage) -> io::Result<Vec<u8>> {
    if image.width != LCD_WIDTH || image.height != LCD_HEIGHT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "expected {LCD_WIDTH}x{LCD_HEIGHT}, got {}x{}",
                image.width, image.height
            ),
        ));
    }
    single_frame_container(&rgb565_le(image)?, LCD_WIDTH, LCD_HEIGHT)
}
