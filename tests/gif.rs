use gigabyte_lcd::gif::{GifFrame, GifLimits, gif_payload_from_path, normalize_frames};
use gigabyte_lcd::image::RgbImage;
use gigabyte_lcd::protocol::{LCD_HEIGHT, LCD_WIDTH};

fn solid_frame(width: u32, height: u32, color: [u8; 3], delay_ms: u16) -> GifFrame {
    GifFrame {
        image: RgbImage::from_pixels(width, height, vec![color; (width * height) as usize])
            .unwrap(),
        delay_ms,
    }
}

fn indexed_gif(width: u16, height: u16, frame_count: usize) -> Vec<u8> {
    let mut data = Vec::new();
    {
        let palette = &[0, 0, 0, 255, 0, 0];
        let mut encoder = gif::Encoder::new(&mut data, width, height, palette).unwrap();
        for index in 0..frame_count {
            let mut frame = gif::Frame::from_indexed_pixels(width, height, [index as u8 % 2], None);
            frame.delay = 10;
            encoder.write_frame(&frame).unwrap();
        }
    }
    data
}

fn indexed_gif_with_tiny_frame(width: u16, height: u16) -> Vec<u8> {
    let mut data = Vec::new();
    {
        let palette = &[0, 0, 0, 255, 0, 0];
        let mut encoder = gif::Encoder::new(&mut data, width, height, palette).unwrap();
        let mut frame = gif::Frame::from_indexed_pixels(1, 1, [1], None);
        frame.delay = 10;
        encoder.write_frame(&frame).unwrap();
    }
    data
}

#[test]
fn normalization_outputs_lcd_sized_frames_without_cropping_or_upscaling() {
    let limits = GifLimits::default();
    let frames = vec![solid_frame(640, 170, [255, 0, 0], 10)];

    let normalized = normalize_frames(frames, &limits).unwrap();

    assert_eq!(normalized.frames.len(), 1);
    assert_eq!(normalized.frames[0].image.width, LCD_WIDTH);
    assert_eq!(normalized.frames[0].image.height, LCD_HEIGHT);
    assert_eq!(normalized.frames[0].delay_ms, 100);
    assert!(normalized.frames[0].image.pixels.contains(&[255, 0, 0]));
    assert_eq!(normalized.frames[0].image.pixels[0], [0, 0, 0]);
}

#[test]
fn normalization_uses_panel_safe_frame_budget_and_preserves_duration() {
    let limits = GifLimits::default();
    let frames = (0..39)
        .map(|index| solid_frame(1, 1, [index as u8 + 1, 0, 0], 50))
        .collect::<Vec<_>>();

    let normalized = normalize_frames(frames, &limits).unwrap();

    assert_eq!(normalized.frames.len(), 20);
    assert_eq!(
        normalized
            .frames
            .iter()
            .map(|frame| frame.delay_ms as u32)
            .sum::<u32>(),
        2_000
    );
    let first_non_black = normalized
        .frames
        .first()
        .unwrap()
        .image
        .pixels
        .iter()
        .find(|pixel| **pixel != [0, 0, 0])
        .unwrap();
    let last_non_black = normalized
        .frames
        .last()
        .unwrap()
        .image
        .pixels
        .iter()
        .find(|pixel| **pixel != [0, 0, 0])
        .unwrap();
    assert_eq!(first_non_black[0], 1);
    assert_eq!(last_non_black[0], 39);
}

#[test]
fn normalization_scales_content_to_panel_safe_content_height() {
    let limits = GifLimits::default();
    let frames = vec![solid_frame(278, 278, [255, 0, 0], 100)];

    let normalized = normalize_frames(frames, &limits).unwrap();
    let non_black_rows = normalized.frames[0]
        .image
        .pixels
        .chunks(LCD_WIDTH as usize)
        .filter(|row| row.iter().any(|pixel| *pixel != [0, 0, 0]))
        .count();

    assert_eq!(non_black_rows, 150);
}

#[test]
fn normalization_reduces_frames_until_payload_is_below_five_mebibytes() {
    let limits = GifLimits {
        max_payload_bytes: 1_000_000,
        ..GifLimits::default()
    };
    let frames = (0..20)
        .map(|index| {
            let pixels = (0..(320 * 170))
                .map(|pixel| {
                    let value = (pixel + index) as u8;
                    [value, value.wrapping_mul(3), value.wrapping_mul(7)]
                })
                .collect();
            GifFrame {
                image: RgbImage::from_pixels(320, 170, pixels).unwrap(),
                delay_ms: 50,
            }
        })
        .collect::<Vec<_>>();

    let normalized = normalize_frames(frames, &limits).unwrap();

    assert!(normalized.payload.len() <= limits.max_payload_bytes);
    assert!(normalized.frames.len() < 20);
}

#[test]
fn file_decoder_accepts_standard_indexed_gif_and_normalizes_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gif");
    {
        let mut file = std::fs::File::create(&path).unwrap();
        let palette = &[0, 0, 0, 255, 0, 0];
        let mut encoder = gif::Encoder::new(&mut file, 2, 1, palette).unwrap();
        let mut frame = gif::Frame::from_indexed_pixels(2, 1, [0, 1], None);
        frame.delay = 1;
        encoder.write_frame(&frame).unwrap();
    }

    let normalized = gif_payload_from_path(&path, &GifLimits::default()).unwrap();

    assert_eq!(normalized.frames.len(), 1);
    assert_eq!(normalized.frames[0].delay_ms, 100);
    assert_eq!(normalized.frames[0].image.width, LCD_WIDTH);
    assert_eq!(normalized.frames[0].image.height, LCD_HEIGHT);
    assert!(!normalized.payload.is_empty());
}

#[test]
fn file_decoder_rejects_gifs_above_source_frame_budget() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("too-many-frames.gif");
    std::fs::write(&path, indexed_gif(1, 1, 25)).unwrap();

    let error = match gif_payload_from_path(&path, &GifLimits::default()) {
        Ok(_) => panic!("expected oversized source frame count to be rejected"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("too many source frames"));
}

#[test]
fn file_decoder_rejects_oversized_logical_screen_before_canvas_allocation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("huge-screen.gif");
    std::fs::write(&path, indexed_gif_with_tiny_frame(2000, 2000)).unwrap();

    let error = gif_payload_from_path(&path, &GifLimits::default()).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("logical screen"));
}
