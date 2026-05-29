use gigabyte_lcd::rgb_protocol::{
    RGB_EX_PACKET_SIZE, RgbLedSetting, build_rgb_ex_led_set_packets, build_rgb_ex_sync_packet,
    rgb_ex_effect_id,
};

fn setting(area: u8, ui_effect: u8, color: u32) -> RgbLedSetting {
    RgbLedSetting {
        area,
        ui_effect,
        speed: area.wrapping_add(1),
        brightness: area.wrapping_add(2),
        color,
        angle: area.wrapping_add(3),
        global: false,
        color_count: 8,
        color_array: (0..8).map(|idx| color + idx).collect(),
        fan_color_array: Vec::new(),
    }
}

fn fan_colors() -> Vec<u32> {
    (0..3)
        .flat_map(|group| (0..8).map(move |idx| ((group + 1) << 20) | idx))
        .collect()
}

#[test]
fn rgb_ex_effect_mapping_matches_gcc_special_cases() {
    assert_eq!(rgb_ex_effect_id(0x20, 0), 0);
    assert_eq!(rgb_ex_effect_id(0x20, 3), 5);
    assert_eq!(rgb_ex_effect_id(0x20, 9), 12);
    assert_eq!(rgb_ex_effect_id(0x20, 11), 9);
    assert_eq!(rgb_ex_effect_id(0x12, 11), 10);
    assert_eq!(rgb_ex_effect_id(0x16, 11), 10);
    assert_eq!(rgb_ex_effect_id(0x20, 13), 10);
    assert_eq!(rgb_ex_effect_id(0x20, 99), 0);
}

#[test]
fn rgb_ex_sync_packet_preserves_gcc_reserved_gap_byte() {
    let packet = build_rgb_ex_sync_packet(0x1234_5678);

    assert_eq!(packet.len(), RGB_EX_PACKET_SIZE);
    assert_eq!(
        &packet[..8],
        &[0x16, 0x01, 0x12, 0x06, 0x00, 0x34, 0x56, 0x78]
    );
    assert!(packet[8..].iter().all(|byte| *byte == 0));
}

#[test]
fn rgb_ex_led_set_for_current_card_reorders_fan_groups_and_falls_back_last_region() {
    let mut fan = setting(2, 1, 0x0a0b0c);
    fan.fan_color_array = fan_colors();
    let settings = vec![
        setting(0, 1, 0x010203),
        fan,
        setting(5, 2, 0x111213),
        setting(6, 3, 0x212223),
    ];

    let packets = build_rgb_ex_led_set_packets(0x20, &settings).unwrap();

    assert_eq!(packets.len(), 6);
    assert!(
        packets
            .iter()
            .all(|packet| packet.len() == RGB_EX_PACKET_SIZE)
    );

    assert_eq!(packets[0][9], 0);
    assert_eq!(packets[1][9], 1);
    assert_eq!(packets[2][9], 2);
    assert_eq!(packets[0][10], 8);
    assert_eq!(packets[1][10], 8);
    assert_eq!(packets[2][10], 8);
    assert_eq!(&packets[0][11..14], &[0x30, 0x00, 0x00]);
    assert_eq!(&packets[1][11..14], &[0x10, 0x00, 0x00]);
    assert_eq!(&packets[2][11..14], &[0x20, 0x00, 0x00]);

    assert_eq!(&packets[3][2..10], &[2, 6, 7, 0x11, 0x12, 0x13, 8, 3]);
    assert_eq!(&packets[4][2..10], &[5, 7, 8, 0x21, 0x22, 0x23, 9, 4]);
    assert_eq!(&packets[5][2..10], &[5, 7, 8, 0x21, 0x22, 0x23, 9, 5]);
}

#[test]
fn rgb_ex_led_set_rejects_short_fan_color_arrays_for_current_card() {
    let settings = vec![setting(0, 1, 0x010203), setting(2, 1, 0x0a0b0c)];

    let error = build_rgb_ex_led_set_packets(0x20, &settings).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(
        error
            .to_string()
            .contains("fan_color_array must contain at least 24 colors")
    );
}
