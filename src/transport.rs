use crate::logging;
use crate::protocol::I2C_PAGE_SIZE;
use crate::rmapi::NvRmI2cTransport;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_ulong};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const I2C_RDWR: c_ulong = 0x0707;
const I2C_M_RD: u16 = 0x0001;

#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

#[repr(C)]
struct I2cRdwrIoctlData {
    msgs: *mut I2cMsg,
    nmsgs: u32,
}

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

pub trait Transport {
    fn write(&self, payload: &[u8]) -> io::Result<()>;

    fn write_read(&self, _payload: &[u8], _read_len: usize) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "transport does not implement write_read",
        ))
    }

    fn write_read_at(&self, _addr: u16, payload: &[u8], read_len: usize) -> io::Result<Vec<u8>> {
        self.write_read(payload, read_len)
    }
}

#[derive(Clone, Debug)]
pub struct LinuxI2cTransport {
    path: PathBuf,
    addr: u16,
    retries: usize,
    retry_delay: Duration,
}

impl LinuxI2cTransport {
    pub fn new(bus: u8, addr: u16) -> Self {
        Self::with_path(PathBuf::from(format!("/dev/i2c-{bus}")), addr)
    }

    pub fn with_path(path: impl Into<PathBuf>, addr: u16) -> Self {
        Self {
            path: path.into(),
            addr,
            retries: 8,
            retry_delay: Duration::from_millis(250),
        }
    }

    fn rdwr(&self, messages: &mut [MessageBuffer]) -> io::Result<Vec<Vec<u8>>> {
        self.rdwr_at(self.addr, messages)
    }

    fn rdwr_at(&self, addr: u16, messages: &mut [MessageBuffer]) -> io::Result<Vec<Vec<u8>>> {
        let file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        let mut ioctl_messages: Vec<I2cMsg> = messages
            .iter_mut()
            .map(|message| I2cMsg {
                addr,
                flags: message.flags,
                len: message.buffer.len() as u16,
                buf: message.buffer.as_mut_ptr(),
            })
            .collect();
        let mut ioctl_data = I2cRdwrIoctlData {
            msgs: ioctl_messages.as_mut_ptr(),
            nmsgs: ioctl_messages.len() as u32,
        };
        let result = unsafe { ioctl(file.as_raw_fd(), I2C_RDWR, &mut ioctl_data) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(messages
            .iter()
            .map(|message| message.buffer.clone())
            .collect())
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
        Err(last_error.unwrap_or_else(|| io::Error::other("I2C retry loop had no attempts")))
    }
}

struct MessageBuffer {
    flags: u16,
    buffer: Vec<u8>,
}

impl Transport for LinuxI2cTransport {
    fn write(&self, payload: &[u8]) -> io::Result<()> {
        if payload.len() > I2C_PAGE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "I2C payload exceeds one page",
            ));
        }
        logging::debug(format!(
            "i2c write path={} addr=0x{:02x} len={} head={}",
            self.path.display(),
            self.addr,
            payload.len(),
            format_head(payload)
        ));
        self.retry(|| {
            self.rdwr(&mut [MessageBuffer {
                flags: 0,
                buffer: payload.to_vec(),
            }])
            .map(|_| ())
        })
    }

    fn write_read(&self, payload: &[u8], read_len: usize) -> io::Result<Vec<u8>> {
        validate_write_read(payload, read_len)?;
        logging::debug(format!(
            "i2c write-read path={} addr=0x{:02x} write_len={} read_len={} head={}",
            self.path.display(),
            self.addr,
            payload.len(),
            read_len,
            format_head(payload)
        ));
        self.retry(|| {
            self.rdwr(&mut [
                MessageBuffer {
                    flags: 0,
                    buffer: payload.to_vec(),
                },
                MessageBuffer {
                    flags: I2C_M_RD,
                    buffer: vec![0; read_len],
                },
            ])
            .map(|buffers| buffers[1].clone())
        })
    }

    fn write_read_at(&self, addr: u16, payload: &[u8], read_len: usize) -> io::Result<Vec<u8>> {
        validate_write_read(payload, read_len)?;
        logging::debug(format!(
            "i2c write-read path={} addr=0x{:02x} write_len={} read_len={} head={}",
            self.path.display(),
            addr,
            payload.len(),
            read_len,
            format_head(payload)
        ));
        self.retry(|| {
            self.rdwr_at(
                addr,
                &mut [
                    MessageBuffer {
                        flags: 0,
                        buffer: payload.to_vec(),
                    },
                    MessageBuffer {
                        flags: I2C_M_RD,
                        buffer: vec![0; read_len],
                    },
                ],
            )
            .map(|buffers| buffers[1].clone())
        })
    }
}

/// Runtime-selected transport: the RM API path (configurable bus speed, used
/// by default to avoid frametime hitches) or the plain i2c-dev fallback.
pub enum AnyTransport {
    I2cDev(LinuxI2cTransport),
    Rm(NvRmI2cTransport),
}

impl Transport for AnyTransport {
    fn write(&self, payload: &[u8]) -> io::Result<()> {
        match self {
            Self::I2cDev(transport) => transport.write(payload),
            Self::Rm(transport) => transport.write(payload),
        }
    }

    fn write_read(&self, payload: &[u8], read_len: usize) -> io::Result<Vec<u8>> {
        match self {
            Self::I2cDev(transport) => transport.write_read(payload, read_len),
            Self::Rm(transport) => transport.write_read(payload, read_len),
        }
    }

    fn write_read_at(&self, addr: u16, payload: &[u8], read_len: usize) -> io::Result<Vec<u8>> {
        match self {
            Self::I2cDev(transport) => transport.write_read_at(addr, payload, read_len),
            Self::Rm(transport) => transport.write_read_at(addr, payload, read_len),
        }
    }
}

pub(crate) fn format_head(payload: &[u8]) -> String {
    payload
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_write_read(payload: &[u8], read_len: usize) -> io::Result<()> {
    if payload.len() > I2C_PAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "I2C write payload exceeds one page",
        ));
    }
    if read_len > u16::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "I2C read length exceeds kernel message limit",
        ));
    }
    Ok(())
}
