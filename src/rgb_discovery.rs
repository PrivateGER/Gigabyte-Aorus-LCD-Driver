use crate::transport::Transport;
use std::fmt;
use std::io;

pub use crate::rgb_protocol::RGB_EX_LINUX_ADDR_CANDIDATE;

pub const N50_NATIVE_LINUX_ADDR_CANDIDATE: u16 = 0x71;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendKind {
    RgbExFirmware,
    RgbExSsid,
    N50Native,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RgbExFirmware => formatter.write_str("RGB Ex firmware"),
            Self::RgbExSsid => formatter.write_str("RGB Ex SSID"),
            Self::N50Native => formatter.write_str("N50 native"),
        }
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeAddresses {
    pub rgb_ex: u16,
    pub n50_native: u16,
}

impl Default for ProbeAddresses {
    fn default() -> Self {
        Self {
            rgb_ex: RGB_EX_LINUX_ADDR_CANDIDATE,
            n50_native: N50_NATIVE_LINUX_ADDR_CANDIDATE,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeResult {
    pub backend: BackendKind,
    pub address: u16,
    pub request: Vec<u8>,
    pub read_len: usize,
    pub response: Option<Vec<u8>>,
    pub decoded: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryReport {
    pub results: Vec<ProbeResult>,
}

impl DiscoveryReport {
    pub fn any_success(&self) -> bool {
        self.results.iter().any(|result| result.response.is_some())
    }
}

impl fmt::Display for DiscoveryReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "RGB discovery report")?;
        for result in &self.results {
            writeln!(
                formatter,
                "- {} addr=0x{:02x} request={} read_len={}",
                result.backend,
                result.address,
                format_bytes(&result.request),
                result.read_len
            )?;
            if let Some(response) = &result.response {
                writeln!(formatter, "  response={}", format_bytes(response))?;
            }
            if let Some(decoded) = &result.decoded {
                writeln!(formatter, "  decoded={decoded}")?;
            }
            if let Some(error) = &result.error {
                writeln!(formatter, "  error={error}")?;
            }
        }
        Ok(())
    }
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

pub fn build_probe_plan(addresses: ProbeAddresses) -> ProbePlan {
    let mut plan = build_default_probe_plan();
    for request in &mut plan.requests {
        request.address = match request.backend {
            BackendKind::RgbExFirmware | BackendKind::RgbExSsid => addresses.rgb_ex,
            BackendKind::N50Native => addresses.n50_native,
        };
    }
    plan
}

pub fn run_rgb_discovery(transport: &impl Transport, addresses: ProbeAddresses) -> DiscoveryReport {
    let mut results = Vec::new();
    for request in build_probe_plan(addresses).requests {
        let result =
            match transport.write_read_at(request.address, &request.write, request.read_len) {
                Ok(response) => {
                    let decoded = decode_response(request.backend, &response)
                        .unwrap_or_else(|error| format!("decode warning: {error}"));
                    ProbeResult {
                        backend: request.backend,
                        address: request.address,
                        request: request.write,
                        read_len: request.read_len,
                        response: Some(response),
                        decoded: Some(decoded),
                        error: None,
                    }
                }
                Err(error) => ProbeResult {
                    backend: request.backend,
                    address: request.address,
                    request: request.write,
                    read_len: request.read_len,
                    response: None,
                    decoded: None,
                    error: Some(error.to_string()),
                },
            };
        results.push(result);
    }
    DiscoveryReport { results }
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

fn decode_response(backend: BackendKind, response: &[u8]) -> io::Result<String> {
    match backend {
        BackendKind::RgbExFirmware => {
            let info = parse_rgb_ex_firmware_response(response)?;
            Ok(format!(
                "RGB Ex firmware version {}, {}",
                info.version,
                if info.ex_4n { "Ex-4N" } else { "legacy marker" }
            ))
        }
        BackendKind::RgbExSsid => {
            let info = parse_rgb_ex_ssid_response(response)?;
            Ok(format!("RGB Ex SSID 0x{:04x}", info.ssid))
        }
        BackendKind::N50Native => {
            let info = parse_n50_native_response(response)?;
            let led_id = info
                .full_led_id
                .map(|id| format!("0x{id:04x}"))
                .unwrap_or_else(|| "unknown".to_string());
            Ok(format!(
                "N50 native firmware {}, tag 0x{:02x}, full LED id {}",
                info.version, info.led_tag, led_id
            ))
        }
    }
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

fn format_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "(empty)".to_string();
    }
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
