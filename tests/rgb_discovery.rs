use gigabyte_lcd::rgb_discovery::{
    BackendKind, N50_NATIVE_LINUX_ADDR_CANDIDATE, RGB_EX_LINUX_ADDR_CANDIDATE,
    build_default_probe_plan, parse_n50_native_response, parse_rgb_ex_firmware_response,
    parse_rgb_ex_ssid_response,
};

#[test]
fn default_probe_plan_is_read_only_and_uses_recovered_candidate_addresses() {
    let plan = build_default_probe_plan();

    assert_eq!(plan.requests.len(), 3);
    assert!(plan.requests.iter().all(|request| request.read_len > 0));
    assert_eq!(plan.requests[0].backend, BackendKind::RgbExFirmware);
    assert_eq!(plan.requests[0].address, RGB_EX_LINUX_ADDR_CANDIDATE);
    assert_eq!(plan.requests[0].write, [0x10, 0x01]);
    assert_eq!(plan.requests[0].read_len, 4);
    assert_eq!(plan.requests[1].backend, BackendKind::RgbExSsid);
    assert_eq!(plan.requests[1].write, [0x11, 0x01]);
    assert_eq!(plan.requests[1].read_len, 4);
    assert_eq!(plan.requests[2].backend, BackendKind::N50Native);
    assert_eq!(plan.requests[2].address, N50_NATIVE_LINUX_ADDR_CANDIDATE);
    assert_eq!(plan.requests[2].write, [0xab, 0x00, 0x00, 0x00]);
    assert_eq!(plan.requests[2].read_len, 8);
}

#[test]
fn rgb_ex_firmware_response_decodes_ex_4n_marker_and_version_bytes() {
    let parsed = parse_rgb_ex_firmware_response(&[0x00, 0x02, 0x01, 0x23]).unwrap();

    assert!(parsed.ex_4n);
    assert_eq!(parsed.version, "1.35");
}

#[test]
fn rgb_ex_ssid_response_decodes_big_endian_ssid() {
    let parsed = parse_rgb_ex_ssid_response(&[0x00, 0x00, 0x41, 0x8c]).unwrap();

    assert_eq!(parsed.ssid, 0x418c);
}

#[test]
fn n50_native_response_decodes_led_id_tag_and_version_nibbles() {
    let parsed = parse_n50_native_response(&[0xab, 0x14, 0x00, 0x13, 0, 0, 0, 0]).unwrap();

    assert_eq!(parsed.version, "1.4");
    assert_eq!(parsed.led_tag, 0x13);
    assert_eq!(parsed.full_led_id, Some(0x1020));
}

#[test]
fn short_and_unrecognized_responses_return_decode_errors() {
    assert!(parse_rgb_ex_firmware_response(&[0x00, 0x02, 0x01]).is_err());
    assert!(parse_rgb_ex_ssid_response(&[0x00, 0x00, 0x41]).is_err());
    assert!(parse_n50_native_response(&[0xaa, 0x14, 0x00, 0x13, 0, 0, 0, 0]).is_err());
}
