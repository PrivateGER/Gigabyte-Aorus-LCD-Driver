use gigabyte_lcd::protocol::{
    DisplayMode, ImageKind, MetricValues, TemplateKind, animation_container,
    build_image_template_packet, build_loop_packet, build_metric_overlay_packet,
    build_metric_values_packet, build_set_mode_packet, build_upload_header_packet,
    build_upload_start_packet, page_count_for_gcc, rle_compress_rgb565_le, single_frame_container,
};

#[test]
fn metric_overlay_packet_uses_vendor_flag_order_and_interval() {
    let packet = build_metric_overlay_packet(0x85, 4);

    assert_eq!(packet.len(), 256);
    assert_eq!(&packet[..5], &[0xe1, 0xcb, 0x55, 0xac, 0x38]);
    assert_eq!(&packet[5..13], &[1, 0, 1, 0, 0, 0, 0, 1]);
    assert_eq!(packet[13], 4);
    assert!(packet[14..].iter().all(|byte| *byte == 0));
}

#[test]
fn metric_values_packet_matches_gcc_big_endian_field_layout() {
    let packet = build_metric_values_packet(MetricValues {
        temperature_c: 48,
        gpu_clock_mhz: 2827,
        gpu_usage_percent: 64,
        fan_rpm: 1514,
        memory_clock_mhz: 15001,
        memory_usage_percent: 33,
        fps: 121,
        power_watts: 211,
    });

    assert_eq!(packet.len(), 256);
    assert_eq!(
        &packet[..20],
        &[
            0xe3, 0xcb, 0x55, 0xac, 0x38, 48, 0x0b, 0x0b, 64, 0x05, 0xea, 0x3a, 0x99, 33, 0x00,
            0x79, 0x00, 0xd3, 0, 0,
        ]
    );
}

#[test]
fn image_upload_header_targets_fifty_series_static_image_slot() {
    let packet = build_upload_header_packet(108_812, ImageKind::Image, 0x21, 0, 0);

    assert_eq!(packet.len(), 256);
    assert_eq!(&packet[..5], &[0xf1, 0xcb, 0x55, 0xac, 0x38]);
    assert_eq!(&packet[5..9], &[0x01, 0x30, 0x00, 0x00]);
    assert_eq!(packet[9], 1);
    assert_eq!(&packet[10..14], &[0, 0, 0x01, 0xaa]);
    assert_eq!(&packet[14..18], &[0, 0, 0, 2]);
}

#[test]
fn gcc_page_count_includes_extra_page_for_exact_multiples() {
    assert_eq!(page_count_for_gcc(0), 1);
    assert_eq!(page_count_for_gcc(1), 1);
    assert_eq!(page_count_for_gcc(255), 1);
    assert_eq!(page_count_for_gcc(256), 2);
    assert_eq!(page_count_for_gcc(108_812), 426);
}

#[test]
fn single_frame_container_uses_little_endian_header_offsets() {
    let pixels = vec![0x34; 320 * 170 * 2];
    let payload = single_frame_container(&pixels, 320, 170).unwrap();

    assert_eq!(payload.len(), 2 + 10 + pixels.len());
    assert_eq!(&payload[0..2], &[1, 0]);
    assert_eq!(&payload[2..6], &(payload.len() as u32 - 1).to_le_bytes());
    assert_eq!(&payload[6..8], &320u16.to_le_bytes());
    assert_eq!(&payload[8..10], &170u16.to_le_bytes());
    assert_eq!(&payload[10..12], &1u16.to_le_bytes());
    assert_eq!(&payload[12..16], &[0x34, 0x34, 0x34, 0x34]);
}

#[test]
fn setup_packets_keep_image_mode_stable_without_saving() {
    let start = build_upload_start_packet();
    let loop_packet = build_loop_packet(&[DisplayMode::Image], 1);
    let mode_packet = build_set_mode_packet(DisplayMode::Image);

    assert_eq!(&start[..6], &[0xf2, 0xcb, 0x55, 0xac, 0x38, 1]);
    assert_eq!(&loop_packet[..7], &[0xf3, 0xcb, 0x55, 0xac, 0x38, 1, 4]);
    assert_eq!(&mode_packet[..6], &[0xe5, 0xcb, 0x55, 0xac, 0x38, 4]);
}

#[test]
fn image_template_packet_matches_gcc_default_gif_overlay_layout() {
    let packet = build_image_template_packet(TemplateKind::Gif, true);

    assert_eq!(packet.len(), 256);
    assert_eq!(
        &packet[..18],
        &[
            0xea, 0xcb, 0x55, 0xac, 0x38, 1, 0xff, 0xff, 0xff, 0, 0, 1, 0x40, 0, 0x92, 0, 0x40, 1,
        ]
    );
    assert!(packet[18..].iter().all(|byte| *byte == 0));
}

#[test]
fn rle_compressor_uses_vendor_literal_and_repeat_blocks() {
    let pixels = [0x1111u16, 0x2222, 0x3333, 0x3333, 0x3333, 0x4444]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();

    assert_eq!(
        rle_compress_rgb565_le(&pixels).unwrap(),
        vec![
            0x02, 0x00, 0x11, 0x11, 0x22, 0x22, 0x03, 0x80, 0x33, 0x33, 0x01, 0x00, 0x44, 0x44,
        ]
    );
}

#[test]
fn animation_container_records_frame_offsets_and_rle_streams() {
    let frame_a = vec![0x00; 320 * 170 * 2];
    let frame_b = vec![0xff; 320 * 170 * 2];

    let payload = animation_container(&[frame_a, frame_b], 320, 170).unwrap();

    assert_eq!(&payload[0..2], &2u16.to_le_bytes());
    assert_eq!(&payload[6..8], &320u16.to_le_bytes());
    assert_eq!(&payload[8..10], &170u16.to_le_bytes());
    assert_eq!(&payload[10..12], &3u16.to_le_bytes());
    assert_eq!(&payload[16..18], &320u16.to_le_bytes());
    assert_eq!(&payload[18..20], &170u16.to_le_bytes());
    assert_eq!(&payload[20..22], &3u16.to_le_bytes());
    assert_eq!(payload.len(), 2 + 20 + 8 + 8);
}
