use std::io;

pub const RGB_EX_PACKET_SIZE: usize = 64;
pub const RGB_EX_LINUX_ADDR_CANDIDATE: u16 = 0x75;
pub const RGB_EX_GCC_SAVE_PORT: u8 = 0xea;
pub const RGB_EX_WRITE_SPEED: u32 = 50;
pub const RGB_EX_READ_SPEED: u32 = 100;

pub type RgbExPacket = [u8; RGB_EX_PACKET_SIZE];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RgbLedSetting {
    pub area: u8,
    pub ui_effect: u8,
    pub speed: u8,
    pub brightness: u8,
    pub color: u32,
    pub angle: u8,
    pub global: bool,
    pub color_count: u8,
    pub color_array: Vec<u32>,
    pub fan_color_array: Vec<u32>,
}

pub fn rgb_ex_effect_id(simple_led_id: u8, ui_effect_id: u8) -> u8 {
    match ui_effect_id {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 5,
        4 => 3,
        5 => 7,
        6 => 8,
        7 => 6,
        8 => 4,
        9 => 12,
        10 => 11,
        11 if matches!(simple_led_id, 18 | 22) => 10,
        11 => 9,
        12 | 13 => 10,
        _ => 0,
    }
}

pub fn build_rgb_ex_sync_packet(color: u32) -> RgbExPacket {
    let mut packet = [0; RGB_EX_PACKET_SIZE];
    packet[0] = 0x16;
    packet[1] = 0x01;
    packet[2] = ((color >> 24) & 0xff) as u8;
    packet[3] = 0x06;
    packet[5] = ((color >> 16) & 0xff) as u8;
    packet[6] = ((color >> 8) & 0xff) as u8;
    packet[7] = (color & 0xff) as u8;
    packet
}

pub fn build_rgb_ex_led_set_packets(
    simple_led_id: u8,
    settings: &[RgbLedSetting],
) -> io::Result<Vec<RgbExPacket>> {
    let settings = settings
        .iter()
        .filter(|setting| setting.area < 100)
        .collect::<Vec<_>>();
    if settings.is_empty() {
        return Err(invalid_input("at least one RGB LED setting is required"));
    }
    if !uses_current_six_packet_layout(simple_led_id) {
        return Err(invalid_input(format!(
            "unsupported RGB Ex simple LED id 0x{simple_led_id:02x}"
        )));
    }

    let global = settings[0].global;
    let mut packets = Vec::with_capacity(6);
    for packet_index in 0..6 {
        let selector = match packet_index {
            0..=2 => 1,
            3 => 2,
            4 => 3,
            _ => 4,
        };
        let setting = if global {
            settings[0]
        } else {
            setting_for_selector(&settings, selector)
        };
        let wire_effect = rgb_ex_effect_id(simple_led_id, setting.ui_effect);
        let color = if setting.ui_effect == 0 {
            0
        } else {
            setting.color
        };
        let mut packet = build_led_set_base_packet(wire_effect, setting, color, packet_index);

        if selector == 1 && matches!(wire_effect, 1..=4) {
            copy_fan_group(&mut packet, setting, packet_index)?;
        } else if matches!(wire_effect, 8 | 9 | 10 | 12) {
            copy_color_array(&mut packet, setting)?;
        }

        packets.push(packet);
    }

    Ok(packets)
}

fn uses_current_six_packet_layout(simple_led_id: u8) -> bool {
    matches!(simple_led_id, 21 | 24 | 25 | 32 | 33 | 34 | 35)
}

fn setting_for_selector<'a>(
    settings: &'a [&'a RgbLedSetting],
    selector: usize,
) -> &'a RgbLedSetting {
    settings
        .get(selector)
        .copied()
        .unwrap_or_else(|| settings[settings.len() - 1])
}

fn build_led_set_base_packet(
    wire_effect: u8,
    setting: &RgbLedSetting,
    color: u32,
    packet_index: u8,
) -> RgbExPacket {
    let mut packet = [0; RGB_EX_PACKET_SIZE];
    packet[0] = 0x12;
    packet[1] = 0x01;
    packet[2] = wire_effect;
    packet[3] = setting.speed;
    packet[4] = setting.brightness;
    write_rgb(color, &mut packet[5..8]);
    packet[8] = setting.angle;
    packet[9] = packet_index;
    packet
}

fn copy_fan_group(
    packet: &mut RgbExPacket,
    setting: &RgbLedSetting,
    packet_index: u8,
) -> io::Result<()> {
    if setting.fan_color_array.len() < 24 {
        return Err(invalid_input(
            "fan_color_array must contain at least 24 colors",
        ));
    }
    let source_group = match packet_index {
        0 => 2,
        1 => 0,
        2 => 1,
        _ => return Ok(()),
    };
    let start = source_group * 8;
    packet[10] = 8;
    write_rgb_triples(packet, &setting.fan_color_array[start..start + 8])
}

fn copy_color_array(packet: &mut RgbExPacket, setting: &RgbLedSetting) -> io::Result<()> {
    let count = setting.color_count as usize;
    if setting.color_array.len() < count {
        return Err(invalid_input(format!(
            "color_array must contain at least {count} colors"
        )));
    }
    packet[10] = setting.color_count;
    write_rgb_triples(packet, &setting.color_array[..count])
}

fn write_rgb_triples(packet: &mut RgbExPacket, colors: &[u32]) -> io::Result<()> {
    let byte_count = colors.len() * 3;
    if 11 + byte_count > RGB_EX_PACKET_SIZE {
        return Err(invalid_input(format!(
            "{} RGB colors exceed one RGB Ex packet",
            colors.len()
        )));
    }
    for (index, color) in colors.iter().copied().enumerate() {
        let start = 11 + index * 3;
        write_rgb(color, &mut packet[start..start + 3]);
    }
    Ok(())
}

fn write_rgb(color: u32, target: &mut [u8]) {
    target[0] = ((color >> 16) & 0xff) as u8;
    target[1] = ((color >> 8) & 0xff) as u8;
    target[2] = (color & 0xff) as u8;
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
