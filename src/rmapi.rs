//! Userspace NVIDIA RM API I2C transport.
//!
//! The in-kernel `/dev/i2c-N` path always clocks GPU I2C transactions at
//! 100 kHz and holds the RM API + GPU locks for the whole transfer, so one
//! 256-byte panel page blocks frame presentation for ~27 ms. The Windows
//! driver requests 400 kHz per transaction through NvAPI_I2CWriteEx, which is
//! why Gigabyte Control Center does not hitch there. This module speaks the
//! same RM control the Windows path ends up in
//! (`NV402C_CTRL_CMD_I2C_TRANSACTION`) directly via the open-gpu-kernel-modules
//! ioctl ABI on `/dev/nvidiactl`, with a configurable bus speed.
//!
//! Struct layouts and ioctl numbers were verified against the
//! NVIDIA/open-gpu-kernel-modules tag matching driver 610.43.02; the layout
//! tests below pin the C ground truth.

use crate::logging;
use crate::protocol::I2C_PAGE_SIZE;
use crate::transport::Transport;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_ulong};
use std::thread;
use std::time::Duration;

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

const NVIDIA_CTL_PATH: &str = "/dev/nvidiactl";

const NV_IOCTL_MAGIC: c_ulong = b'F' as c_ulong;
const NV_ESC_CARD_INFO: c_ulong = 200;
const NV_ESC_CHECK_VERSION_STR: c_ulong = 210;
const NV_ESC_ATTACH_GPUS_TO_FD: c_ulong = 212;
const NV_ESC_RM_FREE: c_ulong = 0x29;
const NV_ESC_RM_CONTROL: c_ulong = 0x2a;
const NV_ESC_RM_ALLOC: c_ulong = 0x2b;

const NV01_ROOT: u32 = 0x0;
const NV01_DEVICE_0: u32 = 0x80;
const NV20_SUBDEVICE_0: u32 = 0x2080;
const NV40_I2C: u32 = 0x402c;

const NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2: u32 = 0x205;
const NV402C_CTRL_CMD_I2C_TRANSACTION: u32 = 0x402c_0105;
const NV402C_CTRL_I2C_TRANSACTION_TYPE_I2C_BLOCK_RW: u32 = 2;
const NV402C_CTRL_I2C_MESSAGE_LENGTH_MAX: usize = 4096;
const NV_RM_API_VERSION_CMD_QUERY: u32 = b'2' as u32;

/// NV_MAX_DEVICES from nvlimits.h.
const MAX_CARDS: usize = 32;

const DEVICE_HANDLE: u32 = 0x5c1d_0080;
const SUBDEVICE_HANDLE: u32 = 0x5c1d_2080;
const I2C_HANDLE: u32 = 0x5c1d_402c;

const fn iowr(nr: c_ulong, size: usize) -> c_ulong {
    (3 << 30) | ((size as c_ulong) << 16) | (NV_IOCTL_MAGIC << 8) | nr
}

/// NV402C_CTRL_I2C_FLAGS_SPEED_MODE occupies flag bits 4:1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum I2cSpeed {
    Khz100 = 0,
    Khz200 = 1,
    Khz400 = 2,
    Khz300 = 7,
}

impl I2cSpeed {
    pub fn from_khz(khz: u16) -> Option<Self> {
        match khz {
            100 => Some(Self::Khz100),
            200 => Some(Self::Khz200),
            300 => Some(Self::Khz300),
            400 => Some(Self::Khz400),
            _ => None,
        }
    }

    fn transaction_flags(self) -> u32 {
        (self as u32) << 1
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NvPciInfo {
    domain: u32,
    bus: u8,
    slot: u8,
    function: u8,
    vendor_id: u16,
    device_id: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NvIoctlCardInfo {
    valid: u8,
    pci_info: NvPciInfo,
    gpu_id: u32,
    interrupt_line: u16,
    reg_address: u64,
    reg_size: u64,
    fb_address: u64,
    fb_size: u64,
    minor_number: u32,
    dev_name: [u8; 10],
}

impl Default for NvIoctlCardInfo {
    fn default() -> Self {
        // SAFETY: the struct is plain integers; all-zero is a valid value.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
struct NvIoctlRmApiVersion {
    cmd: u32,
    reply: u32,
    version_string: [u8; 64],
}

/// NVOS21_PARAMETERS.
#[repr(C)]
#[derive(Default)]
struct NvOs21Parameters {
    h_root: u32,
    h_object_parent: u32,
    h_object_new: u32,
    h_class: u32,
    p_alloc_parms: u64,
    params_size: u32,
    status: u32,
}

/// NVOS54_PARAMETERS.
#[repr(C)]
#[derive(Default)]
struct NvOs54Parameters {
    h_client: u32,
    h_object: u32,
    cmd: u32,
    flags: u32,
    params: u64,
    params_size: u32,
    status: u32,
}

/// NVOS00_PARAMETERS.
#[repr(C)]
#[derive(Default)]
struct NvOs00Parameters {
    h_root: u32,
    h_object_parent: u32,
    h_object_old: u32,
    status: u32,
}

/// NV0080_ALLOC_PARAMETERS.
#[repr(C)]
#[derive(Default)]
struct Nv0080AllocParameters {
    device_id: u32,
    h_client_share: u32,
    h_target_client: u32,
    h_target_device: u32,
    flags: u32,
    _pad: u32,
    va_space_size: u64,
    va_start_internal: u64,
    va_limit_internal: u64,
    va_mode: u32,
    _pad2: u32,
}

/// NV2080_ALLOC_PARAMETERS.
#[repr(C)]
#[derive(Default)]
struct Nv2080AllocParameters {
    sub_device_id: u32,
}

/// NV0000_CTRL_GPU_GET_ID_INFO_V2_PARAMS.
#[repr(C)]
#[derive(Default)]
struct NvGpuIdInfoV2Params {
    gpu_id: u32,
    gpu_flags: u32,
    device_instance: u32,
    sub_device_instance: u32,
    sli_status: u32,
    board_id: u32,
    gpu_instance: u32,
    numa_id: i32,
}

/// NV402C_CTRL_I2C_TRANSACTION_PARAMS with the transData union flattened to
/// its I2C_BLOCK_RW member; the union itself is 80 bytes.
#[repr(C)]
struct Nv402cI2cTransactionParams {
    port_id: u8,
    _pad0: [u8; 3],
    flags: u32,
    device_address: u16,
    _pad1: [u8; 2],
    trans_type: u32,
    b_write: u8,
    _pad2: [u8; 3],
    message_length: u32,
    p_message: u64,
    _union_pad: [u8; 64],
}

/// NVIDIA i2c adapter identity parsed from the sysfs adapter name, e.g.
/// `NVIDIA i2c adapter 1 at 1:00.0` (RM port number, PCI bus:slot.function).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterInfo {
    pub port: u8,
    pub pci_bus: u8,
    pub pci_slot: u8,
}

pub fn parse_nvidia_adapter_name(name: &str) -> Option<AdapterInfo> {
    let rest = name.trim().strip_prefix("NVIDIA i2c adapter ")?;
    let (port_text, rest) = rest.split_once(" at ")?;
    let port = port_text.parse::<u8>().ok()?;
    let (bus_text, rest) = rest.split_once(':')?;
    let (slot_text, _function) = rest.split_once('.')?;
    let pci_bus = u8::from_str_radix(bus_text, 16).ok()?;
    let pci_slot = u8::from_str_radix(slot_text, 16).ok()?;
    Some(AdapterInfo {
        port,
        pci_bus,
        pci_slot,
    })
}

fn adapter_info_for_bus(bus: u8) -> io::Result<AdapterInfo> {
    let path = format!("/sys/bus/i2c/devices/i2c-{bus}/name");
    let name = fs::read_to_string(&path)?;
    parse_nvidia_adapter_name(&name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "i2c bus {bus} is not an NVIDIA GPU adapter (name: {})",
                name.trim()
            ),
        )
    })
}

pub struct NvRmI2cTransport {
    control: File,
    /// Open handle on /dev/nvidia{minor}. The RM only lets a user client
    /// allocate a device if the process holds the GPU's device node open
    /// (nv_is_gpu_accessible scans the process fd table), and the transactions
    /// need the adapter to stay initialized.
    _device: File,
    client: u32,
    addr: u16,
    port: u8,
    speed: I2cSpeed,
    retries: usize,
    retry_delay: Duration,
}

impl NvRmI2cTransport {
    /// Opens the RM transport for the GPU that owns Linux i2c bus `bus`.
    ///
    /// The bus number is only used to look up the NVIDIA adapter name in
    /// sysfs, which carries both the RM port number and the GPU PCI address.
    pub fn open(bus: u8, addr: u16, speed: I2cSpeed) -> io::Result<Self> {
        let adapter = adapter_info_for_bus(bus)?;
        let control = OpenOptions::new()
            .read(true)
            .write(true)
            .open(NVIDIA_CTL_PATH)?;
        let version = query_driver_version(&control)?;
        let card = find_card(&control, &adapter)?;
        let gpu_id = card.gpu_id;
        let device = OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/dev/nvidia{}", card.minor_number))?;

        let mut transport = Self {
            control,
            _device: device,
            client: 0,
            addr,
            port: adapter.port,
            speed,
            retries: 8,
            retry_delay: Duration::from_millis(250),
        };

        transport.attach_gpu(gpu_id)?;
        transport.client = transport.rm_alloc(0, 0, 0, NV01_ROOT, None)?;
        let device_instance = transport.gpu_device_instance(gpu_id)?;
        let mut device_params = Nv0080AllocParameters {
            device_id: device_instance,
            h_client_share: transport.client,
            ..Default::default()
        };
        transport.rm_alloc(
            transport.client,
            transport.client,
            DEVICE_HANDLE,
            NV01_DEVICE_0,
            Some((
                &mut device_params as *mut _ as u64,
                size_of::<Nv0080AllocParameters>(),
            )),
        )?;
        let mut subdevice_params = Nv2080AllocParameters::default();
        transport.rm_alloc(
            transport.client,
            DEVICE_HANDLE,
            SUBDEVICE_HANDLE,
            NV20_SUBDEVICE_0,
            Some((
                &mut subdevice_params as *mut _ as u64,
                size_of::<Nv2080AllocParameters>(),
            )),
        )?;
        transport.rm_alloc(
            transport.client,
            SUBDEVICE_HANDLE,
            I2C_HANDLE,
            NV40_I2C,
            None,
        )?;

        logging::info(format!(
            "RM I2C transport ready: driver {version}, gpu id 0x{gpu_id:08x}, \
             port {}, addr 0x{addr:02x}, speed {:?}",
            adapter.port, speed
        ));
        Ok(transport)
    }

    fn ioctl_raw(&self, request: c_ulong, data: *mut std::ffi::c_void) -> io::Result<()> {
        control_ioctl(&self.control, request, data)
    }

    fn attach_gpu(&self, gpu_id: u32) -> io::Result<()> {
        let mut gpu_ids = [gpu_id];
        self.ioctl_raw(
            iowr(NV_ESC_ATTACH_GPUS_TO_FD, size_of::<[u32; 1]>()),
            gpu_ids.as_mut_ptr() as *mut _,
        )
    }

    fn rm_alloc(
        &self,
        root: u32,
        parent: u32,
        handle: u32,
        class: u32,
        params: Option<(u64, usize)>,
    ) -> io::Result<u32> {
        let (p_alloc_parms, params_size) = params.unwrap_or((0, 0));
        let mut alloc = NvOs21Parameters {
            h_root: root,
            h_object_parent: parent,
            h_object_new: handle,
            h_class: class,
            p_alloc_parms,
            params_size: params_size as u32,
            status: 0,
        };
        self.ioctl_raw(
            iowr(NV_ESC_RM_ALLOC, size_of::<NvOs21Parameters>()),
            &mut alloc as *mut _ as *mut _,
        )?;
        if alloc.status != 0 {
            return Err(rm_status_error("NV_ESC_RM_ALLOC", class, alloc.status));
        }
        Ok(alloc.h_object_new)
    }

    fn rm_control(&self, object: u32, cmd: u32, params: u64, params_size: usize) -> io::Result<()> {
        let mut control = NvOs54Parameters {
            h_client: self.client,
            h_object: object,
            cmd,
            flags: 0,
            params,
            params_size: params_size as u32,
            status: 0,
        };
        self.ioctl_raw(
            iowr(NV_ESC_RM_CONTROL, size_of::<NvOs54Parameters>()),
            &mut control as *mut _ as *mut _,
        )?;
        if control.status != 0 {
            return Err(rm_status_error("NV_ESC_RM_CONTROL", cmd, control.status));
        }
        Ok(())
    }

    fn gpu_device_instance(&self, gpu_id: u32) -> io::Result<u32> {
        let mut params = NvGpuIdInfoV2Params {
            gpu_id,
            ..Default::default()
        };
        self.rm_control(
            self.client,
            NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2,
            &mut params as *mut _ as u64,
            size_of::<NvGpuIdInfoV2Params>(),
        )?;
        Ok(params.device_instance)
    }

    fn transact(&self, addr: u16, write: bool, buffer: &mut [u8]) -> io::Result<()> {
        let mut params = Nv402cI2cTransactionParams {
            port_id: self.port,
            _pad0: [0; 3],
            flags: self.speed.transaction_flags(),
            // RM expects the address preshifted into 8-bit form.
            device_address: addr << 1,
            _pad1: [0; 2],
            trans_type: NV402C_CTRL_I2C_TRANSACTION_TYPE_I2C_BLOCK_RW,
            b_write: u8::from(write),
            _pad2: [0; 3],
            message_length: buffer.len() as u32,
            p_message: buffer.as_mut_ptr() as u64,
            _union_pad: [0; 64],
        };
        self.rm_control(
            I2C_HANDLE,
            NV402C_CTRL_CMD_I2C_TRANSACTION,
            &mut params as *mut _ as u64,
            size_of::<Nv402cI2cTransactionParams>(),
        )
    }

    fn retry<T>(&self, mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        let mut last_error = None;
        for attempt in 0..self.retries {
            match operation() {
                Ok(value) => return Ok(value),
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 < self.retries {
                        thread::sleep(self.retry_delay);
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| io::Error::other("RM I2C retry loop had no attempts")))
    }

    fn write_then_read_at(
        &self,
        addr: u16,
        payload: &[u8],
        read_len: usize,
    ) -> io::Result<Vec<u8>> {
        validate_lengths(payload.len(), read_len)?;
        // I2C_BLOCK_RW carries a single direction per call, so this is a
        // write transaction followed by a read transaction rather than a
        // combined repeated-start cycle. The Windows driver's GvReadI2C does
        // the same (NvAPI_I2CWriteEx then NvAPI_I2CReadEx), and the GPU bus
        // has no other master that could interpose between the two.
        self.retry(|| {
            let mut write_buffer = payload.to_vec();
            self.transact(addr, true, &mut write_buffer)?;
            let mut read_buffer = vec![0; read_len];
            self.transact(addr, false, &mut read_buffer)?;
            Ok(read_buffer)
        })
    }
}

impl Drop for NvRmI2cTransport {
    fn drop(&mut self) {
        if self.client == 0 {
            return;
        }
        // For NV01_ROOT the RM treats the client as its own parent; freeing
        // the root client cascades to the device/subdevice/i2c children.
        let mut free = NvOs00Parameters {
            h_root: self.client,
            h_object_parent: self.client,
            h_object_old: self.client,
            status: 0,
        };
        let _ = self.ioctl_raw(
            iowr(NV_ESC_RM_FREE, size_of::<NvOs00Parameters>()),
            &mut free as *mut _ as *mut _,
        );
    }
}

impl Transport for NvRmI2cTransport {
    fn write(&self, payload: &[u8]) -> io::Result<()> {
        if payload.len() > I2C_PAGE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "I2C payload exceeds one page",
            ));
        }
        logging::debug(format!(
            "rm i2c write port={} addr=0x{:02x} len={} speed={:?} head={}",
            self.port,
            self.addr,
            payload.len(),
            self.speed,
            crate::transport::format_head(payload)
        ));
        self.retry(|| {
            let mut buffer = payload.to_vec();
            self.transact(self.addr, true, &mut buffer)
        })
    }

    fn write_read(&self, payload: &[u8], read_len: usize) -> io::Result<Vec<u8>> {
        self.write_read_at(self.addr, payload, read_len)
    }

    fn write_read_at(&self, addr: u16, payload: &[u8], read_len: usize) -> io::Result<Vec<u8>> {
        logging::debug(format!(
            "rm i2c write-read port={} addr=0x{:02x} write_len={} read_len={} speed={:?}",
            self.port,
            addr,
            payload.len(),
            read_len,
            self.speed
        ));
        self.write_then_read_at(addr, payload, read_len)
    }
}

fn control_ioctl(control: &File, request: c_ulong, data: *mut std::ffi::c_void) -> io::Result<()> {
    let result = unsafe { ioctl(control.as_raw_fd(), request, data) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn query_driver_version(control: &File) -> io::Result<String> {
    let mut params = NvIoctlRmApiVersion {
        cmd: NV_RM_API_VERSION_CMD_QUERY,
        reply: 0,
        version_string: [0; 64],
    };
    control_ioctl(
        control,
        iowr(NV_ESC_CHECK_VERSION_STR, size_of::<NvIoctlRmApiVersion>()),
        &mut params as *mut _ as *mut _,
    )?;
    let len = params
        .version_string
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(params.version_string.len());
    Ok(String::from_utf8_lossy(&params.version_string[..len]).into_owned())
}

fn find_card(control: &File, adapter: &AdapterInfo) -> io::Result<NvIoctlCardInfo> {
    let mut cards = [NvIoctlCardInfo::default(); MAX_CARDS];
    control_ioctl(
        control,
        iowr(NV_ESC_CARD_INFO, size_of::<[NvIoctlCardInfo; MAX_CARDS]>()),
        cards.as_mut_ptr() as *mut _,
    )?;
    cards
        .iter()
        .find(|card| {
            card.valid != 0
                && card.pci_info.bus == adapter.pci_bus
                && card.pci_info.slot == adapter.pci_slot
        })
        .copied()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no NVIDIA GPU at PCI {:x}:{:02x} in RM card list",
                    adapter.pci_bus, adapter.pci_slot
                ),
            )
        })
}

fn validate_lengths(write_len: usize, read_len: usize) -> io::Result<()> {
    if write_len > I2C_PAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "I2C write payload exceeds one page",
        ));
    }
    if read_len > NV402C_CTRL_I2C_MESSAGE_LENGTH_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "I2C read length exceeds RM message limit",
        ));
    }
    Ok(())
}

fn rm_status_error(call: &str, subject: u32, status: u32) -> io::Error {
    io::Error::other(format!(
        "{call} (0x{subject:x}) failed with NV status 0x{status:02x}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    // Ground truth sizes/offsets compiled from the open-gpu-kernel-modules
    // 610.43.02 headers with gcc on x86-64.

    #[test]
    fn ioctl_numbers_match_kernel_encoding() {
        assert_eq!(iowr(NV_ESC_RM_CONTROL, 32), 0xc020_462a);
        assert_eq!(iowr(NV_ESC_RM_ALLOC, 32), 0xc020_462b);
        assert_eq!(iowr(NV_ESC_RM_FREE, 16), 0xc010_4629);
        assert_eq!(iowr(NV_ESC_CHECK_VERSION_STR, 72), 0xc048_46d2);
        assert_eq!(iowr(NV_ESC_ATTACH_GPUS_TO_FD, 4), 0xc004_46d4);
        assert_eq!(iowr(NV_ESC_CARD_INFO, 72 * MAX_CARDS), 0xc900_46c8);
    }

    #[test]
    fn ffi_struct_layouts_match_kernel_abi() {
        assert_eq!(size_of::<NvPciInfo>(), 12);
        assert_eq!(offset_of!(NvPciInfo, bus), 4);
        assert_eq!(offset_of!(NvPciInfo, slot), 5);
        assert_eq!(offset_of!(NvPciInfo, vendor_id), 8);

        assert_eq!(size_of::<NvIoctlCardInfo>(), 72);
        assert_eq!(offset_of!(NvIoctlCardInfo, pci_info), 4);
        assert_eq!(offset_of!(NvIoctlCardInfo, gpu_id), 16);
        assert_eq!(offset_of!(NvIoctlCardInfo, interrupt_line), 20);
        assert_eq!(offset_of!(NvIoctlCardInfo, reg_address), 24);
        assert_eq!(offset_of!(NvIoctlCardInfo, minor_number), 56);

        assert_eq!(size_of::<NvIoctlRmApiVersion>(), 72);
        assert_eq!(size_of::<NvOs21Parameters>(), 32);
        assert_eq!(offset_of!(NvOs21Parameters, p_alloc_parms), 16);
        assert_eq!(size_of::<NvOs54Parameters>(), 32);
        assert_eq!(offset_of!(NvOs54Parameters, params), 16);
        assert_eq!(size_of::<NvOs00Parameters>(), 16);

        assert_eq!(size_of::<Nv0080AllocParameters>(), 56);
        assert_eq!(offset_of!(Nv0080AllocParameters, flags), 16);
        assert_eq!(offset_of!(Nv0080AllocParameters, va_space_size), 24);
        assert_eq!(offset_of!(Nv0080AllocParameters, va_mode), 48);
        assert_eq!(size_of::<Nv2080AllocParameters>(), 4);
        assert_eq!(size_of::<NvGpuIdInfoV2Params>(), 32);

        assert_eq!(size_of::<Nv402cI2cTransactionParams>(), 96);
        assert_eq!(offset_of!(Nv402cI2cTransactionParams, flags), 4);
        assert_eq!(offset_of!(Nv402cI2cTransactionParams, device_address), 8);
        assert_eq!(offset_of!(Nv402cI2cTransactionParams, trans_type), 12);
        assert_eq!(offset_of!(Nv402cI2cTransactionParams, b_write), 16);
        assert_eq!(offset_of!(Nv402cI2cTransactionParams, message_length), 20);
        assert_eq!(offset_of!(Nv402cI2cTransactionParams, p_message), 24);
    }

    #[test]
    fn speed_modes_encode_into_flag_bits_4_to_1() {
        assert_eq!(I2cSpeed::Khz100.transaction_flags(), 0x0);
        assert_eq!(I2cSpeed::Khz200.transaction_flags(), 0x2);
        assert_eq!(I2cSpeed::Khz400.transaction_flags(), 0x4);
        assert_eq!(I2cSpeed::Khz300.transaction_flags(), 0xe);
        assert_eq!(I2cSpeed::from_khz(400), Some(I2cSpeed::Khz400));
        assert_eq!(I2cSpeed::from_khz(150), None);
        assert_eq!(I2cSpeed::from_khz(0), None);
    }

    #[test]
    fn parses_real_nvidia_adapter_name() {
        let info = parse_nvidia_adapter_name("NVIDIA i2c adapter 1 at 1:00.0\n").unwrap();

        assert_eq!(
            info,
            AdapterInfo {
                port: 1,
                pci_bus: 1,
                pci_slot: 0,
            }
        );
    }

    #[test]
    fn parses_hex_pci_bus_in_adapter_name() {
        let info = parse_nvidia_adapter_name("NVIDIA i2c adapter 6 at 2f:0a.0").unwrap();

        assert_eq!(
            info,
            AdapterInfo {
                port: 6,
                pci_bus: 0x2f,
                pci_slot: 0x0a,
            }
        );
    }

    #[test]
    fn rejects_foreign_and_malformed_adapter_names() {
        assert_eq!(
            parse_nvidia_adapter_name("SMBus PIIX4 adapter port 0 at 0b00"),
            None
        );
        assert_eq!(parse_nvidia_adapter_name("NVIDIA SOC i2c adapter 2"), None);
        assert_eq!(
            parse_nvidia_adapter_name("NVIDIA i2c adapter 999 at 1:00.0"),
            None
        );
        assert_eq!(
            parse_nvidia_adapter_name("NVIDIA i2c adapter x at 1:00.0"),
            None
        );
        assert_eq!(
            parse_nvidia_adapter_name("NVIDIA i2c adapter 1 at zz:00.0"),
            None
        );
        assert_eq!(parse_nvidia_adapter_name(""), None);
    }

    #[test]
    fn rejects_oversized_transfers() {
        assert!(validate_lengths(I2C_PAGE_SIZE, 4).is_ok());
        assert!(validate_lengths(I2C_PAGE_SIZE + 1, 4).is_err());
        assert!(validate_lengths(16, NV402C_CTRL_I2C_MESSAGE_LENGTH_MAX + 1).is_err());
    }
}
