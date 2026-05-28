use std::fmt;
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LogLevel {
    Info = 1,
    Debug = 2,
}

static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

impl LogLevel {
    pub fn parse(value: &str) -> io::Result<Self> {
        match value {
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid log level {value:?}; expected info or debug"),
            )),
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => formatter.write_str("info"),
            Self::Debug => formatter.write_str("debug"),
        }
    }
}

pub fn set_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

fn enabled(level: LogLevel) -> bool {
    LOG_LEVEL.load(Ordering::Relaxed) >= level as u8
}

pub fn info(message: impl fmt::Display) {
    if enabled(LogLevel::Info) {
        eprintln!("INFO {message}");
    }
}

pub fn debug(message: impl fmt::Display) {
    if enabled(LogLevel::Debug) {
        eprintln!("DEBUG {message}");
    }
}
