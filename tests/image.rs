use gigabyte_lcd::image::{RgbImage, mascot_background, rgb565_le};

fn rgba_png(width: u32, height: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
    let mut data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let raw: Vec<u8> = pixels.iter().flat_map(|px| px.iter().copied()).collect();
        writer.write_image_data(&raw).unwrap();
    }
    data
}

#[test]
fn rgb565_conversion_is_little_endian() {
    let image = RgbImage::from_pixels(
        2,
        2,
        vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]],
    )
    .unwrap();

    assert_eq!(
        rgb565_le(&image).unwrap(),
        vec![0x00, 0xf8, 0xe0, 0x07, 0x1f, 0x00, 0xff, 0xff]
    );
}

#[test]
fn mascot_background_places_alpha_composited_image_on_left_side() {
    let mut pixels = vec![[0, 0, 0, 0]; 20 * 2];
    pixels[8] = [255, 0, 0, 255];
    pixels[9] = [0, 255, 0, 128];
    pixels[28] = [0, 0, 255, 255];
    let png = rgba_png(20, 2, &pixels);

    let rendered = mascot_background(&png).unwrap();

    assert_eq!(rendered.width, 320);
    assert_eq!(rendered.height, 170);
    assert!(rendered.pixels.contains(&[255, 0, 0]));
    assert!(
        rendered
            .pixels
            .iter()
            .any(|pixel| pixel[1] > 0 && pixel[1] < 255)
    );
    assert_eq!(rendered.pixels[319], [0, 0, 0]);
}
