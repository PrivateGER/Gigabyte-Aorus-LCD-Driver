use std::io;

pub const LCD_WIDTH: u32 = 320;
pub const LCD_HEIGHT: u32 = 170;
pub const I2C_PAGE_SIZE: usize = 256;
pub const DEFAULT_BUS: u8 = 1;
pub const DEFAULT_ADDR: u16 = 0x61;
pub const DEFAULT_DEVICE_LED_ID: u8 = 0x21;
const GCC_MAGIC: [u8; 4] = [0xcb, 0x55, 0xac, 0x38];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ImageKind {
    Gif = 0,
    Image = 1,
    Text = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayMode {
    Faith1 = 0,
    Faith2 = 1,
    Faith3 = 2,
    Image = 3,
    Text = 4,
    Gif = 5,
    ChibiTime = 6,
    Carousel = 7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TemplateKind {
    Gif = 1,
    Image = 2,
    Pet = 3,
}

/// Bit positions in the 0xE1 metric overlay flags, matching the value order
/// of the 0xE3 metric values packet.
pub const METRIC_FLAG_TEMPERATURE: u8 = 1 << 0;
pub const METRIC_FLAG_GPU_CLOCK: u8 = 1 << 1;
pub const METRIC_FLAG_GPU_USAGE: u8 = 1 << 2;
pub const METRIC_FLAG_FAN: u8 = 1 << 3;
pub const METRIC_FLAG_MEMORY_CLOCK: u8 = 1 << 4;
pub const METRIC_FLAG_MEMORY_USAGE: u8 = 1 << 5;
pub const METRIC_FLAG_FPS: u8 = 1 << 6;
pub const METRIC_FLAG_POWER: u8 = 1 << 7;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MetricValues {
    pub temperature_c: u16,
    pub gpu_clock_mhz: u32,
    pub gpu_usage_percent: u16,
    pub fan_rpm: u32,
    pub memory_clock_mhz: u32,
    pub memory_usage_percent: u16,
    pub fps: u32,
    pub power_watts: u32,
}

pub fn padded_packet(data: &[u8]) -> io::Result<Vec<u8>> {
    if data.len() > I2C_PAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("packet is too large: {} > {}", data.len(), I2C_PAGE_SIZE),
        ));
    }
    let mut packet = vec![0; I2C_PAGE_SIZE];
    packet[..data.len()].copy_from_slice(data);
    Ok(packet)
}

pub fn gcc_command(opcode: u8, args: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + GCC_MAGIC.len() + args.len());
    data.push(opcode);
    data.extend_from_slice(&GCC_MAGIC);
    data.extend_from_slice(args);
    padded_packet(&data).expect("GCC command builders must not exceed one I2C page")
}

pub fn clamp_u8(value: u16) -> u8 {
    value.min(u8::MAX as u16) as u8
}

pub fn clamp_u16(value: u32) -> u16 {
    value.min(u16::MAX as u32) as u16
}

fn mode_wire_arg(mode: DisplayMode) -> u8 {
    let mode_value = match mode {
        DisplayMode::Carousel => 9,
        other => other as u8,
    };
    mode_value + 1
}

pub fn build_open_packet(enabled: bool) -> Vec<u8> {
    gcc_command(0xe7, &[if enabled { 1 } else { 2 }])
}

pub fn build_set_mode_packet(mode: DisplayMode) -> Vec<u8> {
    gcc_command(0xe5, &[mode_wire_arg(mode)])
}

pub fn build_save_packet() -> Vec<u8> {
    gcc_command(0xaa, &[])
}

pub fn build_metric_overlay_packet(flags: u8, interval: u8) -> Vec<u8> {
    let mut args = Vec::with_capacity(9);
    for bit in 0..8 {
        args.push(if flags & (1 << bit) != 0 { 1 } else { 0 });
    }
    args.push(interval.max(1));
    gcc_command(0xe1, &args)
}

pub fn build_image_template_packet(kind: TemplateKind, enabled: bool) -> Vec<u8> {
    const DEFAULT_COLOR_RGB: [u8; 3] = [0xff, 0xff, 0xff];
    const DEFAULT_IMAGE_POS: (u16, u16) = (0, 320);
    const DEFAULT_DATA_POS: (u16, u16) = (146, 64);

    let mut args = Vec::with_capacity(13);
    args.push(kind as u8);
    args.extend_from_slice(&DEFAULT_COLOR_RGB);
    args.extend_from_slice(&DEFAULT_IMAGE_POS.0.to_be_bytes());
    args.extend_from_slice(&DEFAULT_IMAGE_POS.1.to_be_bytes());
    args.extend_from_slice(&DEFAULT_DATA_POS.0.to_be_bytes());
    args.extend_from_slice(&DEFAULT_DATA_POS.1.to_be_bytes());
    args.push(u8::from(enabled));
    gcc_command(0xea, &args)
}

pub fn build_metric_values_packet(values: MetricValues) -> Vec<u8> {
    let gpu_clock = clamp_u16(values.gpu_clock_mhz).to_be_bytes();
    let fan = clamp_u16(values.fan_rpm).to_be_bytes();
    let memory_clock = clamp_u16(values.memory_clock_mhz).to_be_bytes();
    let fps = clamp_u16(values.fps).to_be_bytes();
    let power = clamp_u16(values.power_watts).to_be_bytes();
    let args = [
        clamp_u8(values.temperature_c),
        gpu_clock[0],
        gpu_clock[1],
        clamp_u8(values.gpu_usage_percent),
        fan[0],
        fan[1],
        memory_clock[0],
        memory_clock[1],
        clamp_u8(values.memory_usage_percent),
        fps[0],
        fps[1],
        power[0],
        power[1],
    ];
    gcc_command(0xe3, &args)
}

pub fn build_loop_packet(modes: &[DisplayMode], interval: u8) -> Vec<u8> {
    let mut args = Vec::with_capacity(1 + modes.len());
    args.push(interval.max(1));
    for mode in modes.iter().copied().take(24) {
        if (mode as u8) <= DisplayMode::ChibiTime as u8 {
            args.push(mode as u8 + 1);
        }
    }
    gcc_command(0xf3, &args)
}

fn image_target_address(kind: ImageKind, device_led_id: u8) -> (u32, u8) {
    let fifty_series = matches!(device_led_id, 0x18 | 0x19 | 0x20 | 0x21 | 0x22);
    match kind {
        ImageKind::Image => (
            if fifty_series {
                0x0130_0000
            } else {
                0x01f2_6000
            },
            1,
        ),
        ImageKind::Text => (
            if fifty_series {
                0x0132_0000
            } else {
                0x01f0_0000
            },
            1,
        ),
        ImageKind::Gif => (0, 2),
    }
}

pub fn page_count_for_gcc(byte_count: usize) -> u32 {
    (byte_count / I2C_PAGE_SIZE + 1) as u32
}

pub fn upload_chunk_mode(byte_count: usize) -> (u8, usize, std::time::Duration) {
    if byte_count < 20_480 {
        (1, 4096, std::time::Duration::from_millis(400))
    } else {
        (2, 65_536, std::time::Duration::from_secs(2))
    }
}

pub fn build_upload_start_packet() -> Vec<u8> {
    gcc_command(0xf2, &[1])
}

pub fn build_upload_finish_packet() -> Vec<u8> {
    gcc_command(0xf2, &[2])
}

pub fn build_upload_header_packet(
    payload_size: usize,
    kind: ImageKind,
    device_led_id: u8,
    frame_count: u16,
    delay_ms: u8,
) -> Vec<u8> {
    let (target_address, storage_type) = image_target_address(kind, device_led_id);
    let page_count = page_count_for_gcc(payload_size);
    let (chunk_mode, _, _) = upload_chunk_mode(payload_size);
    let mut args = Vec::with_capacity(13);
    args.extend_from_slice(&target_address.to_be_bytes());
    args.push(storage_type);
    args.extend_from_slice(&page_count.to_be_bytes());
    args.extend_from_slice(&frame_count.to_be_bytes());
    args.push(delay_ms);
    args.push(chunk_mode);
    args.push(0);
    gcc_command(0xf1, &args)
}

pub fn single_frame_container(rgb565_le: &[u8], width: u32, height: u32) -> io::Result<Vec<u8>> {
    let expected = width as usize * height as usize * 2;
    if rgb565_le.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("expected {expected} RGB565 bytes, got {}", rgb565_le.len()),
        ));
    }
    let frame_count = 1u16;
    let header_len = 2 + frame_count as usize * 10;
    let end_offset_minus_one = header_len + rgb565_le.len() - 1;
    let mut output = Vec::with_capacity(header_len + rgb565_le.len());
    output.extend_from_slice(&frame_count.to_le_bytes());
    output.extend_from_slice(&(end_offset_minus_one as u32).to_le_bytes());
    output.extend_from_slice(&(width as u16).to_le_bytes());
    output.extend_from_slice(&(height as u16).to_le_bytes());
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(rgb565_le);
    Ok(output)
}

fn find_rle_block(values: &[u16], start: usize) -> (usize, usize) {
    let segment_end = values.len().min(start + 32_767);
    let segment = &values[start..segment_end];
    if segment.len() < 4 {
        return (segment.len(), 0);
    }

    let mut index = 0;
    while index < segment.len() {
        if index + 2 == segment.len() {
            return (segment.len(), 0);
        }
        if segment[index] == segment[index + 1] && segment[index] == segment[index + 2] {
            let run_start = index;
            index += 2;
            while index < segment.len() - 1 && segment[index] == segment[index + 1] {
                index += 1;
            }
            return (run_start, index + 1 - run_start);
        }
        index += 1;
    }

    (segment.len(), 0)
}

pub fn rle_compress_rgb565_le(rgb565_le: &[u8]) -> io::Result<Vec<u8>> {
    if !rgb565_le.len().is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "RGB565 payload must have an even byte count",
        ));
    }
    let values = rgb565_le
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut index = 0;

    while index < values.len() {
        let (literal_count, repeated_count) = find_rle_block(&values, index);
        if literal_count > 0 {
            output.extend_from_slice(&(literal_count as u16).to_le_bytes());
            for value in &values[index..index + literal_count] {
                output.extend_from_slice(&value.to_le_bytes());
            }
        }
        if repeated_count > 0 {
            output.extend_from_slice(&((repeated_count as u16) | 0x8000).to_le_bytes());
            output.extend_from_slice(&values[index + literal_count].to_le_bytes());
        }
        index += literal_count + repeated_count;
    }

    Ok(output)
}

pub fn animation_container(
    frame_rgb565_le: &[Vec<u8>],
    width: u32,
    height: u32,
) -> io::Result<Vec<u8>> {
    if frame_rgb565_le.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "animation must contain at least one frame",
        ));
    }
    if frame_rgb565_le.len() > u16::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "animation contains too many frames",
        ));
    }
    let expected = width as usize * height as usize * 2;
    let mut streams = Vec::with_capacity(frame_rgb565_le.len());
    for frame in frame_rgb565_le {
        if frame.len() != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("expected {expected} RGB565 bytes, got {}", frame.len()),
            ));
        }
        streams.push(rle_compress_rgb565_le(frame)?);
    }

    let mut offset = 2 + streams.len() * 10;
    let mut headers = Vec::with_capacity(streams.len() * 10);
    for stream in &streams {
        offset += stream.len();
        headers.extend_from_slice(&(offset as u32 - 1).to_le_bytes());
        headers.extend_from_slice(&(width as u16).to_le_bytes());
        headers.extend_from_slice(&(height as u16).to_le_bytes());
        headers.extend_from_slice(&3u16.to_le_bytes());
    }

    let mut output =
        Vec::with_capacity(2 + headers.len() + streams.iter().map(Vec::len).sum::<usize>());
    output.extend_from_slice(&(streams.len() as u16).to_le_bytes());
    output.extend_from_slice(&headers);
    for stream in streams {
        output.extend_from_slice(&stream);
    }
    Ok(output)
}
