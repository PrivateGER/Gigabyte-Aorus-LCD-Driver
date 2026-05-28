use clap::Parser;
use gigabyte_lcd::gif::{
    GifLimits, MAX_GIF_CONTENT_HEIGHT_DEFAULT, MAX_GIF_FRAMES_DEFAULT, MAX_GIF_FRAMES_EXCLUSIVE,
    MIN_GIF_DELAY_MS,
};
use gigabyte_lcd::logging::LogLevel;
use gigabyte_lcd::protocol::DEFAULT_BUS;
use gigabyte_lcd::service::OverlayConfig;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(test)]
use std::io;

#[derive(Debug)]
pub(crate) struct Args {
    pub(crate) mascot: PathBuf,
    pub(crate) gif: Option<PathBuf>,
    pub(crate) gif_limits: GifLimits,
    pub(crate) bus: u8,
    pub(crate) addr: u16,
    pub(crate) device_id: u8,
    pub(crate) image_settle_delay: Duration,
    pub(crate) overlay: OverlayConfig,
    pub(crate) gpu_index: u32,
    pub(crate) log_level: LogLevel,
}

#[derive(Debug, Parser)]
#[command(
    name = "gigabyte-lcd",
    version,
    about = "Linux control service for Gigabyte AORUS GPU LCD panels",
    long_about = None
)]
struct CliArgs {
    #[arg(
        long,
        value_name = "PATH",
        help = "Required PNG to place on the left side"
    )]
    mascot: PathBuf,

    #[arg(
        long,
        value_name = "PATH",
        help = "GIF to normalize and play panel-side"
    )]
    gif: Option<PathBuf>,

    #[arg(
        long = "gif-max-frames",
        default_value_t = MAX_GIF_FRAMES_DEFAULT,
        value_parser = parse_gif_max_frames,
        value_name = "N",
        help = "Maximum normalized GIF frames"
    )]
    gif_max_frames: usize,

    #[arg(
        long = "gif-min-delay-ms",
        default_value_t = MIN_GIF_DELAY_MS,
        value_parser = parse_u16,
        value_name = "N",
        help = "Minimum normalized GIF frame delay"
    )]
    gif_min_delay_ms: u16,

    #[arg(
        long = "gif-content-height",
        default_value_t = MAX_GIF_CONTENT_HEIGHT_DEFAULT,
        value_parser = parse_u32,
        value_name = "N",
        help = "Maximum GIF content height"
    )]
    gif_content_height: u32,

    #[arg(
        long,
        default_value_t = DEFAULT_BUS,
        value_parser = parse_u8,
        value_name = "N",
        help = "Linux I2C bus"
    )]
    bus: u8,

    #[arg(
        long,
        default_value = "0x61",
        value_parser = parse_u16,
        value_name = "ADDR",
        help = "7-bit I2C address"
    )]
    addr: u16,

    #[arg(
        long = "device-id",
        default_value = "0x21",
        value_parser = parse_u8,
        value_name = "ID",
        help = "LCD device LED id"
    )]
    device_id: u8,

    #[arg(
        long = "image-settle-delay",
        default_value = "5",
        value_parser = parse_image_settle_delay,
        value_name = "SEC",
        help = "Extra delay after upload"
    )]
    image_settle_delay: Duration,

    #[arg(
        long = "metrics",
        default_value = "temp,usage,power",
        value_parser = parse_metric_flags,
        value_name = "LIST",
        help = "Overlay metrics"
    )]
    metric_flags: u8,

    #[arg(
        long = "overlay-interval",
        default_value_t = 4,
        value_parser = parse_u8,
        value_name = "N",
        help = "Vendor overlay rotation interval"
    )]
    overlay_interval: u8,

    #[arg(
        long = "gpu-index",
        default_value_t = 0,
        value_parser = parse_u32,
        value_name = "N",
        help = "NVML GPU index"
    )]
    gpu_index: u32,

    #[arg(
        long = "log-level",
        default_value = "info",
        value_parser = parse_log_level,
        value_name = "LEVEL",
        help = "info or debug"
    )]
    log_level: LogLevel,
}

impl Args {
    pub(crate) fn parse_cli() -> Self {
        CliArgs::parse().into()
    }

    #[cfg(test)]
    fn parse(args: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let args = std::iter::once(String::from("gigabyte-lcd")).chain(args);
        CliArgs::try_parse_from(args)
            .map(Self::from)
            .map_err(clap_error_to_io)
    }
}

impl From<CliArgs> for Args {
    fn from(cli: CliArgs) -> Self {
        let mut gif_limits = GifLimits::default();
        gif_limits.max_frames_exclusive = cli.gif_max_frames + 1;
        gif_limits.min_delay_ms = cli.gif_min_delay_ms.max(1);
        gif_limits.max_content_height = cli.gif_content_height.clamp(1, gif_limits.height);

        Self {
            mascot: cli.mascot,
            gif: cli.gif,
            gif_limits,
            bus: cli.bus,
            addr: cli.addr,
            device_id: cli.device_id,
            image_settle_delay: cli.image_settle_delay,
            overlay: OverlayConfig {
                interval: cli.overlay_interval.max(1),
                flags: cli.metric_flags,
            },
            gpu_index: cli.gpu_index,
            log_level: cli.log_level,
        }
    }
}

#[cfg(test)]
fn clap_error_to_io(error: clap::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}

fn parse_gif_max_frames(value: &str) -> Result<usize, String> {
    let max_frames = parse_usize(value)?;
    if max_frames == 0 || max_frames >= MAX_GIF_FRAMES_EXCLUSIVE {
        return Err(format!(
            "--gif-max-frames maximum is {}",
            MAX_GIF_FRAMES_EXCLUSIVE - 1
        ));
    }
    Ok(max_frames)
}

fn parse_u8(value: &str) -> Result<u8, String> {
    let parsed = parse_u16(value)?;
    u8::try_from(parsed).map_err(|_| format!("{value:?} must fit in u8"))
}

fn parse_u16(value: &str) -> Result<u16, String> {
    let text = value.strip_prefix("0x").unwrap_or(value);
    let radix = if value.starts_with("0x") { 16 } else { 10 };
    u16::from_str_radix(text, radix).map_err(|_| format!("invalid integer: {value:?}"))
}

fn parse_u32(value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("invalid integer: {value:?}"))
}

fn parse_usize(value: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("invalid integer: {value:?}"))
}

fn parse_image_settle_delay(value: &str) -> Result<Duration, String> {
    let seconds: f64 = value
        .parse()
        .map_err(|_| format!("invalid --image-settle-delay: {value:?}"))?;
    if !seconds.is_finite() {
        return Err("--image-settle-delay must be finite".to_string());
    }
    if seconds > 3600.0 {
        return Err("--image-settle-delay must be at most 3600 seconds".to_string());
    }
    Ok(Duration::from_secs_f64(seconds.max(0.0)))
}

fn parse_log_level(value: &str) -> Result<LogLevel, String> {
    LogLevel::parse(value).map_err(|error| error.to_string())
}

fn parse_metric_flags(value: &str) -> Result<u8, String> {
    let mut flags = 0u8;
    for raw_name in value.split(',') {
        let name = raw_name.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        let bit = match name.as_str() {
            "temp" | "temperature" | "gpu-temp" | "gpu-temperature" => 0,
            "clock" | "gpu-clock" => 1,
            "usage" | "gpu" | "gpu-usage" => 2,
            "fan" | "fan-speed" => 3,
            "vram-clock" | "memory-clock" | "mem-clock" => 4,
            "vram" | "vram-usage" | "memory" | "memory-usage" | "mem" | "mem-usage" => 5,
            "power" | "pwr" | "tgp" => 7,
            "all" => {
                flags = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 7);
                continue;
            }
            "none" | "off" => {
                flags = 0;
                continue;
            }
            "fps" => return Err(format!("unsupported metric {raw_name:?}")),
            _ => return Err(format!("unknown metric {raw_name:?}")),
        };
        flags |= 1 << bit;
    }
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gigabyte_lcd::protocol::{DEFAULT_ADDR, DEFAULT_DEVICE_LED_ID};

    #[test]
    fn default_settle_delay_is_panel_verified_when_mascot_is_provided() {
        let args = Args::parse(["--mascot", "mascot.png"].into_iter().map(String::from)).unwrap();

        assert_eq!(args.image_settle_delay, Duration::from_secs(5));
    }

    #[test]
    fn rejects_missing_mascot_path() {
        let error = Args::parse(std::iter::empty()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("--mascot"));
    }

    #[test]
    fn parses_gif_limit_overrides() {
        let args = Args::parse(
            [
                "--mascot",
                "mascot.png",
                "--gif",
                "input.gif",
                "--gif-max-frames",
                "30",
                "--gif-min-delay-ms",
                "75",
                "--gif-content-height",
                "160",
            ]
            .into_iter()
            .map(String::from),
        )
        .unwrap();

        assert_eq!(args.gif, Some(PathBuf::from("input.gif")));
        assert_eq!(args.gif_limits.max_frames_exclusive, 31);
        assert_eq!(args.gif_limits.min_delay_ms, 75);
        assert_eq!(args.gif_limits.max_content_height, 160);
    }

    #[test]
    fn parses_hex_device_options() {
        let args = Args::parse(
            [
                "--mascot",
                "mascot.png",
                "--bus",
                "0x02",
                "--addr",
                "0x61",
                "--device-id",
                "0x21",
            ]
            .into_iter()
            .map(String::from),
        )
        .unwrap();

        assert_eq!(args.bus, 2);
        assert_eq!(args.addr, DEFAULT_ADDR);
        assert_eq!(args.device_id, DEFAULT_DEVICE_LED_ID);
    }

    #[test]
    fn parses_metric_selection() {
        let args = Args::parse(
            [
                "--mascot",
                "mascot.png",
                "--metrics",
                "temp,gpu-clock,fan,vram-usage,pwr",
            ]
            .into_iter()
            .map(String::from),
        )
        .unwrap();

        assert_eq!(
            args.overlay.flags,
            (1 << 0) | (1 << 1) | (1 << 3) | (1 << 5) | (1 << 7)
        );
    }

    #[test]
    fn all_metrics_includes_fan_but_not_fps() {
        let args = Args::parse(
            ["--mascot", "mascot.png", "--metrics", "all"]
                .into_iter()
                .map(String::from),
        )
        .unwrap();

        assert_eq!(
            args.overlay.flags,
            (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 7)
        );
    }

    #[test]
    fn rejects_unknown_metric_selection() {
        let error = Args::parse(
            ["--mascot", "mascot.png", "--metrics", "temperature,watts"]
                .into_iter()
                .map(String::from),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("unknown metric"));
    }

    #[test]
    fn rejects_fps_metric_without_a_real_fps_source() {
        let error = Args::parse(
            ["--mascot", "mascot.png", "--metrics", "fps"]
                .into_iter()
                .map(String::from),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("unsupported metric"));
    }

    #[test]
    fn rejects_non_finite_image_settle_delay_without_panicking() {
        let result = std::panic::catch_unwind(|| {
            Args::parse(
                ["--mascot", "mascot.png", "--image-settle-delay", "inf"]
                    .into_iter()
                    .map(String::from),
            )
        });

        assert!(result.is_ok(), "parser should return an error, not panic");
        let error = result.unwrap().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("finite"));
    }

    #[test]
    fn rejects_gif_frame_limit_above_safe_budget() {
        let error = Args::parse(
            ["--mascot", "mascot.png", "--gif-max-frames", "999"]
                .into_iter()
                .map(String::from),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("maximum"));
    }
}
