use crate::protocol::MetricValues;
use crate::service::TelemetrySource;
use std::ffi::{CStr, CString, c_void};
use std::io;
use std::os::raw::{c_char, c_int, c_uint};

const RTLD_NOW: c_int = 2;
const NVML_SUCCESS: c_int = 0;
const NVML_TEMPERATURE_GPU: c_uint = 0;
const NVML_CLOCK_GRAPHICS: c_uint = 0;
const NVML_CLOCK_MEM: c_uint = 2;
const NVML_FAN_SPEED_INFO_V1: c_uint =
    std::mem::size_of::<NvmlFanSpeedInfo>() as c_uint | (1 << 24);

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[repr(C)]
#[derive(Default)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

#[repr(C)]
#[derive(Default)]
struct NvmlUtilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct NvmlFanSpeedInfo {
    version: c_uint,
    fan: c_uint,
    speed: c_uint,
}

type NvmlDevice = *mut c_void;
type NvmlInit = unsafe extern "C" fn() -> c_int;
type NvmlDeviceGetHandleByIndex = unsafe extern "C" fn(c_uint, *mut NvmlDevice) -> c_int;
type NvmlDeviceGetMemoryInfo = unsafe extern "C" fn(NvmlDevice, *mut NvmlMemory) -> c_int;
type NvmlDeviceGetPowerUsage = unsafe extern "C" fn(NvmlDevice, *mut c_uint) -> c_int;
type NvmlDeviceGetTemperature = unsafe extern "C" fn(NvmlDevice, c_uint, *mut c_uint) -> c_int;
type NvmlDeviceGetUtilizationRates =
    unsafe extern "C" fn(NvmlDevice, *mut NvmlUtilization) -> c_int;
type NvmlDeviceGetClockInfo = unsafe extern "C" fn(NvmlDevice, c_uint, *mut c_uint) -> c_int;
type NvmlDeviceGetNumFans = unsafe extern "C" fn(NvmlDevice, *mut c_uint) -> c_int;
type NvmlDeviceGetFanSpeedRpm = unsafe extern "C" fn(NvmlDevice, *mut NvmlFanSpeedInfo) -> c_int;

pub struct NvmlTelemetry {
    _library: *mut c_void,
    device: NvmlDevice,
    symbols: NvmlSymbols,
}

struct NvmlSymbols {
    init: NvmlInit,
    get_handle_by_index: NvmlDeviceGetHandleByIndex,
    get_memory_info: NvmlDeviceGetMemoryInfo,
    get_power_usage: NvmlDeviceGetPowerUsage,
    get_temperature: NvmlDeviceGetTemperature,
    get_utilization_rates: NvmlDeviceGetUtilizationRates,
    get_clock_info: NvmlDeviceGetClockInfo,
    get_num_fans: Option<NvmlDeviceGetNumFans>,
    get_fan_speed_rpm: Option<NvmlDeviceGetFanSpeedRpm>,
}

impl NvmlTelemetry {
    pub fn open(index: u32) -> io::Result<Self> {
        let library_name = CString::new("libnvidia-ml.so.1").unwrap();
        let library = unsafe { dlopen(library_name.as_ptr(), RTLD_NOW) };
        if library.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "could not load libnvidia-ml.so.1",
            ));
        }

        let symbols = NvmlSymbols::load(library)?;
        symbols.initialize()?;
        let device = symbols.device_by_index(index)?;

        Ok(Self {
            _library: library,
            device,
            symbols,
        })
    }
}

impl NvmlSymbols {
    fn load(library: *mut c_void) -> io::Result<Self> {
        Ok(Self {
            init: symbol(library, "nvmlInit_v2")?,
            get_handle_by_index: symbol(library, "nvmlDeviceGetHandleByIndex_v2")?,
            get_memory_info: symbol(library, "nvmlDeviceGetMemoryInfo")?,
            get_power_usage: symbol(library, "nvmlDeviceGetPowerUsage")?,
            get_temperature: symbol(library, "nvmlDeviceGetTemperature")?,
            get_utilization_rates: symbol(library, "nvmlDeviceGetUtilizationRates")?,
            get_clock_info: symbol(library, "nvmlDeviceGetClockInfo")?,
            get_num_fans: optional_symbol(library, "nvmlDeviceGetNumFans"),
            get_fan_speed_rpm: optional_symbol(library, "nvmlDeviceGetFanSpeedRPM"),
        })
    }

    fn initialize(&self) -> io::Result<()> {
        check(unsafe { (self.init)() }, "nvmlInit_v2")
    }

    fn device_by_index(&self, index: u32) -> io::Result<NvmlDevice> {
        let mut device = std::ptr::null_mut();
        check(
            unsafe { (self.get_handle_by_index)(index as c_uint, &mut device) },
            "nvmlDeviceGetHandleByIndex_v2",
        )?;
        Ok(device)
    }

    fn fan_rpm(&self, device: NvmlDevice) -> c_uint {
        let Some(get_fan_speed_rpm) = self.get_fan_speed_rpm else {
            return 0;
        };

        query_fan_rpm(
            || {
                self.get_num_fans.and_then(|get_num_fans| {
                    let mut count = 0;
                    let result = unsafe { get_num_fans(device, &mut count) };
                    if result == NVML_SUCCESS && count > 0 {
                        Some(count)
                    } else {
                        None
                    }
                })
            },
            |_, info| unsafe { get_fan_speed_rpm(device, info) == NVML_SUCCESS },
        )
    }
}

impl TelemetrySource for NvmlTelemetry {
    fn read(&mut self) -> io::Result<MetricValues> {
        let mut memory = NvmlMemory::default();
        let mut power_mw = 0;
        let mut temperature = 0;
        let mut utilization = NvmlUtilization::default();
        let mut graphics_clock = 0;
        let mut memory_clock = 0;
        let fan_rpm = self.symbols.fan_rpm(self.device);

        optional(unsafe { (self.symbols.get_memory_info)(self.device, &mut memory) });
        optional(unsafe { (self.symbols.get_power_usage)(self.device, &mut power_mw) });
        optional(unsafe {
            (self.symbols.get_temperature)(self.device, NVML_TEMPERATURE_GPU, &mut temperature)
        });
        optional(unsafe { (self.symbols.get_utilization_rates)(self.device, &mut utilization) });
        optional(unsafe {
            (self.symbols.get_clock_info)(self.device, NVML_CLOCK_GRAPHICS, &mut graphics_clock)
        });
        optional(unsafe {
            (self.symbols.get_clock_info)(self.device, NVML_CLOCK_MEM, &mut memory_clock)
        });

        let memory_usage_percent = if memory.total > 0 {
            ((memory.used as f64 / memory.total as f64) * 100.0).round() as u16
        } else {
            0
        };

        Ok(MetricValues {
            temperature_c: temperature as u16,
            gpu_clock_mhz: graphics_clock,
            gpu_usage_percent: utilization.gpu as u16,
            fan_rpm,
            memory_clock_mhz: memory_clock,
            memory_usage_percent,
            fps: 0,
            power_watts: ((power_mw as f64) / 1000.0).round() as u32,
        })
    }
}

fn check(result: c_int, name: &str) -> io::Result<()> {
    if result == NVML_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{name} failed with NVML code {result}"
        )))
    }
}

fn optional(result: c_int) {
    let _ = result;
}

fn query_fan_rpm(
    mut get_num_fans: impl FnMut() -> Option<c_uint>,
    mut get_fan_speed_rpm: impl FnMut(c_uint, &mut NvmlFanSpeedInfo) -> bool,
) -> c_uint {
    let fan_count = get_num_fans().filter(|count| *count > 0).unwrap_or(1);
    let mut max_rpm = 0;
    for fan in 0..fan_count {
        let mut info = NvmlFanSpeedInfo {
            version: NVML_FAN_SPEED_INFO_V1,
            fan,
            speed: 0,
        };
        if get_fan_speed_rpm(fan, &mut info) {
            max_rpm = max_rpm.max(info.speed);
        }
    }
    max_rpm
}

fn optional_symbol<T>(library: *mut c_void, name: &str) -> Option<T> {
    symbol(library, name).ok()
}

fn symbol<T>(library: *mut c_void, name: &str) -> io::Result<T> {
    let c_name = CString::new(name).unwrap();
    let pointer = unsafe { dlsym(library, c_name.as_ptr()) };
    if pointer.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "missing NVML symbol {}",
                CStr::from_bytes_with_nul(c_name.as_bytes_with_nul())
                    .unwrap()
                    .to_string_lossy()
            ),
        ));
    }
    Ok(unsafe { std::mem::transmute_copy(&pointer) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_rpm_query_preserves_zero_rpm_reading() {
        let rpm = query_fan_rpm(
            || Some(1),
            |fan, info| {
                assert_eq!(fan, 0);
                assert_eq!(info.version, NVML_FAN_SPEED_INFO_V1);
                info.speed = 0;
                true
            },
        );

        assert_eq!(rpm, 0);
    }

    #[test]
    fn fan_rpm_query_uses_highest_reported_fan_speed() {
        let readings = [900, 1_250, 1_100];
        let rpm = query_fan_rpm(
            || Some(readings.len() as c_uint),
            |fan, info| {
                info.speed = readings[fan as usize];
                true
            },
        );

        assert_eq!(rpm, 1_250);
    }

    #[test]
    fn fan_rpm_query_falls_back_to_fan_zero_when_count_is_unavailable() {
        let mut queried = Vec::new();
        let rpm = query_fan_rpm(
            || None,
            |fan, info| {
                queried.push(fan);
                info.speed = 777;
                true
            },
        );

        assert_eq!(queried, vec![0]);
        assert_eq!(rpm, 777);
    }
}
