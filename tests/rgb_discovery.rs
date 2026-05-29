use gigabyte_lcd::rgb_discovery::{
    BackendKind, N50_NATIVE_LINUX_ADDR_CANDIDATE, ProbeAddresses, RGB_EX_LINUX_ADDR_CANDIDATE,
    build_default_probe_plan, parse_n50_native_response, parse_rgb_ex_firmware_response,
    parse_rgb_ex_ssid_response, run_rgb_discovery,
};
use gigabyte_lcd::transport::Transport;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;

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

#[derive(Default)]
struct FakeProbeTransport {
    calls: RefCell<Vec<(u16, Vec<u8>, usize)>>,
    responses: RefCell<VecDeque<io::Result<Vec<u8>>>>,
}

impl FakeProbeTransport {
    fn with_responses(responses: Vec<io::Result<Vec<u8>>>) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            responses: RefCell::new(responses.into()),
        }
    }
}

impl Transport for FakeProbeTransport {
    fn write(&self, _payload: &[u8]) -> io::Result<()> {
        unreachable!("RGB discovery must not issue write-only transactions")
    }

    fn write_read_at(&self, addr: u16, payload: &[u8], read_len: usize) -> io::Result<Vec<u8>> {
        self.calls
            .borrow_mut()
            .push((addr, payload.to_vec(), read_len));
        self.responses
            .borrow_mut()
            .pop_front()
            .expect("test provided too few fake probe responses")
    }
}

#[test]
fn run_rgb_discovery_uses_per_request_addresses_and_keeps_going_after_errors() {
    let transport = FakeProbeTransport::with_responses(vec![
        Ok(vec![0, 2, 1, 4]),
        Err(io::Error::other("no ack")),
        Ok(vec![0xab, 0x14, 0, 0x13, 0, 0, 0, 0]),
    ]);

    let report = run_rgb_discovery(
        &transport,
        ProbeAddresses {
            rgb_ex: 0x75,
            n50_native: 0x71,
        },
    );

    assert_eq!(
        transport.calls.borrow().as_slice(),
        &[
            (0x75, vec![0x10, 0x01], 4),
            (0x75, vec![0x11, 0x01], 4),
            (0x71, vec![0xab, 0x00, 0x00, 0x00], 8),
        ]
    );
    assert!(report.any_success());
    assert_eq!(report.results.len(), 3);
    assert!(
        report.results[0]
            .decoded
            .as_ref()
            .unwrap()
            .contains("Ex-4N")
    );
    assert!(report.results[1].error.as_ref().unwrap().contains("no ack"));
    assert!(
        report.results[2]
            .decoded
            .as_ref()
            .unwrap()
            .contains("0x1020")
    );
}

#[test]
fn discovery_report_display_includes_probe_details_and_decode_status() {
    let transport = FakeProbeTransport::with_responses(vec![
        Ok(vec![0, 2, 1, 4]),
        Err(io::Error::other("no ack")),
        Ok(vec![0xab, 0x14, 0, 0x13, 0, 0, 0, 0]),
    ]);

    let report = run_rgb_discovery(&transport, ProbeAddresses::default());
    let text = report.to_string();

    assert!(text.contains("RGB Ex firmware"));
    assert!(text.contains("addr=0x75"));
    assert!(text.contains("request=10 01"));
    assert!(text.contains("read_len=4"));
    assert!(text.contains("response=00 02 01 04"));
    assert!(text.contains("error=no ack"));
    assert!(text.contains("N50 native"));
    assert!(text.contains("0x1020"));
}
