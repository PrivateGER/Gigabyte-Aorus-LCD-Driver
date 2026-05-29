use std::io;

pub use crate::rgb_protocol::RGB_EX_LINUX_ADDR_CANDIDATE;

pub const N50_NATIVE_LINUX_ADDR_CANDIDATE: u16 = 0x71;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendKind {
    RgbExFirmware,
    RgbExSsid,
    N50Native,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeRequest {
    pub backend: BackendKind,
    pub address: u16,
    pub write: Vec<u8>,
    pub read_len: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbePlan {
    pub requests: Vec<ProbeRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbExFirmwareInfo {
    pub ex_4n: bool,
    pub version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RgbExSsidInfo {
    pub ssid: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct N50NativeInfo {
    pub version: String,
    pub led_tag: u8,
    pub full_led_id: Option<u16>,
}

pub fn build_default_probe_plan() -> ProbePlan {
    ProbePlan {
        requests: vec![
            ProbeRequest {
                backend: BackendKind::RgbExFirmware,
                address: RGB_EX_LINUX_ADDR_CANDIDATE,
                write: vec![0x10, 0x01],
                read_len: 4,
            },
            ProbeRequest {
                backend: BackendKind::RgbExSsid,
                address: RGB_EX_LINUX_ADDR_CANDIDATE,
                write: vec![0x11, 0x01],
                read_len: 4,
            },
            ProbeRequest {
                backend: BackendKind::N50Native,
                address: N50_NATIVE_LINUX_ADDR_CANDIDATE,
                write: vec![0xab, 0x00, 0x00, 0x00],
                read_len: 8,
            },
        ],
    }
}

pub fn parse_rgb_ex_firmware_response(response: &[u8]) -> io::Result<RgbExFirmwareInfo> {
    if response.len() < 4 {
        return Err(invalid_data(
            "RGB Ex firmware response is shorter than 4 bytes",
        ));
    }

    Ok(RgbExFirmwareInfo {
        ex_4n: response[1] == 2,
        version: format!("{}.{}", response[2], response[3]),
    })
}

pub fn parse_rgb_ex_ssid_response(response: &[u8]) -> io::Result<RgbExSsidInfo> {
    if response.len() < 4 {
        return Err(invalid_data("RGB Ex SSID response is shorter than 4 bytes"));
    }

    Ok(RgbExSsidInfo {
        ssid: u16::from_be_bytes([response[2], response[3]]),
    })
}

pub fn parse_n50_native_response(response: &[u8]) -> io::Result<N50NativeInfo> {
    if response.len() < 4 {
        return Err(invalid_data("N50 native response is shorter than 4 bytes"));
    }
    if response[0] != 0xab {
        return Err(invalid_data(format!(
            "N50 native response marker was 0x{:02x}, expected 0xab",
            response[0]
        )));
    }

    let led_tag = response[3];
    Ok(N50NativeInfo {
        version: format!("{}.{}", response[1] >> 4, response[1] & 0x0f),
        led_tag,
        full_led_id: n50_full_led_id_for_tag(led_tag),
    })
}

fn n50_full_led_id_for_tag(tag: u8) -> Option<u16> {
    match tag {
        0x0b => Some(0x1018),
        0x10 => Some(0x1015),
        0x11 => Some(0x1019),
        0x13 => Some(0x1020),
        0x14 => Some(0x1021),
        _ => None,
    }
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
