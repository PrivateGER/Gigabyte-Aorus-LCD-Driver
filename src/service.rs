use crate::device::{Lcd, UploadOptions};
use crate::logging;
use crate::protocol::{
    DisplayMode, METRIC_FLAG_FAN, METRIC_FLAG_FPS, METRIC_FLAG_GPU_CLOCK, METRIC_FLAG_GPU_USAGE,
    METRIC_FLAG_MEMORY_CLOCK, METRIC_FLAG_MEMORY_USAGE, METRIC_FLAG_POWER, METRIC_FLAG_TEMPERATURE,
    MetricValues, TemplateKind,
};
use crate::transport::Transport;
use std::io;
use std::thread;
use std::time::Duration;

pub const DEFAULT_LCD_INTERVAL_OVERLAY_FLAGS: u8 =
    METRIC_FLAG_TEMPERATURE | METRIC_FLAG_GPU_USAGE | METRIC_FLAG_POWER;

/// Every panel write stalls the GPU for the bus transaction (~6 ms at
/// 400 kHz), so stale-but-equal values are not re-sent. This cap bounds how
/// long the panel can display outdated values if it lost a write (e.g. a
/// panel reset between transactions).
pub const METRIC_FORCE_RESEND_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayConfig {
    pub flags: u8,
    pub interval: u8,
    pub update_interval: Duration,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            flags: DEFAULT_LCD_INTERVAL_OVERLAY_FLAGS,
            interval: 4,
            update_interval: Duration::from_secs(1),
        }
    }
}

/// Decides whether a fresh telemetry sample is worth an I2C transaction.
///
/// A sample is sent when a metric that the overlay actually displays moved by
/// at least its display-noise threshold relative to the *last sent* sample
/// (so slow drift accumulates until it crosses the threshold), or when
/// `force_resend_after` has elapsed since the last write.
pub struct MetricChangeDetector {
    overlay_flags: u8,
    force_resend_after: Duration,
    last_sent: Option<MetricValues>,
    since_last_send: Duration,
}

impl MetricChangeDetector {
    pub fn new(overlay_flags: u8, force_resend_after: Duration) -> Self {
        Self {
            overlay_flags,
            force_resend_after,
            last_sent: None,
            since_last_send: Duration::ZERO,
        }
    }

    /// Records a telemetry sample and returns whether it should be written to
    /// the panel. `elapsed` is the logical tick duration of the caller's loop
    /// (the configured update interval), matching the injected-sleep model
    /// the service uses; the staleness cap is therefore tracked in logical
    /// time and may run one transaction duration behind wall clock.
    pub fn observe(&mut self, values: MetricValues, elapsed: Duration) -> bool {
        self.since_last_send = self.since_last_send.saturating_add(elapsed);
        let send = match &self.last_sent {
            None => true,
            Some(last_sent) => {
                self.since_last_send >= self.force_resend_after
                    || displayed_metrics_changed(self.overlay_flags, last_sent, &values)
            }
        };
        if send {
            self.last_sent = Some(values);
            self.since_last_send = Duration::ZERO;
        }
        send
    }
}

fn displayed_metrics_changed(flags: u8, last: &MetricValues, new: &MetricValues) -> bool {
    // Thresholds sit just above per-second sensor jitter; anything smaller is
    // unreadable on the panel while it rotates metrics every few seconds.
    // Temperature needs >=2 so a reading oscillating across one degree
    // (26<->27) does not trigger a bus write on every flip.
    const TEMPERATURE_C: u16 = 2;
    const GPU_CLOCK_MHZ: u32 = 15;
    const GPU_USAGE_PERCENT: u16 = 2;
    const FAN_RPM: u32 = 50;
    const MEMORY_CLOCK_MHZ: u32 = 15;
    const MEMORY_USAGE_PERCENT: u16 = 2;
    const POWER_WATTS: u32 = 3;

    let displayed = |flag: u8| flags & flag != 0;
    (displayed(METRIC_FLAG_TEMPERATURE)
        && new.temperature_c.abs_diff(last.temperature_c) >= TEMPERATURE_C)
        || (displayed(METRIC_FLAG_GPU_CLOCK)
            && new.gpu_clock_mhz.abs_diff(last.gpu_clock_mhz) >= GPU_CLOCK_MHZ)
        || (displayed(METRIC_FLAG_GPU_USAGE)
            && new.gpu_usage_percent.abs_diff(last.gpu_usage_percent) >= GPU_USAGE_PERCENT)
        || (displayed(METRIC_FLAG_FAN) && new.fan_rpm.abs_diff(last.fan_rpm) >= FAN_RPM)
        || (displayed(METRIC_FLAG_MEMORY_CLOCK)
            && new.memory_clock_mhz.abs_diff(last.memory_clock_mhz) >= MEMORY_CLOCK_MHZ)
        || (displayed(METRIC_FLAG_MEMORY_USAGE)
            && new.memory_usage_percent.abs_diff(last.memory_usage_percent) >= MEMORY_USAGE_PERCENT)
        || (displayed(METRIC_FLAG_FPS) && new.fps != last.fps)
        || (displayed(METRIC_FLAG_POWER)
            && new.power_watts.abs_diff(last.power_watts) >= POWER_WATTS)
}

pub trait TelemetrySource {
    fn read(&mut self) -> io::Result<MetricValues>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayUpload {
    pub payload: Vec<u8>,
    pub options: UploadOptions,
    pub mode: DisplayMode,
    pub static_reset_payload: Option<Vec<u8>>,
}

impl DisplayUpload {
    pub fn image(payload: Vec<u8>) -> Self {
        Self {
            payload,
            options: UploadOptions::image(),
            mode: DisplayMode::Image,
            static_reset_payload: None,
        }
    }

    pub fn gif(payload: Vec<u8>, frame_count: u16, delay_ms: u8) -> Self {
        Self {
            payload,
            options: UploadOptions::gif(frame_count, delay_ms),
            mode: DisplayMode::Gif,
            static_reset_payload: None,
        }
    }

    pub fn with_static_reset(mut self, payload: Vec<u8>) -> Self {
        self.static_reset_payload = Some(payload);
        self
    }
}

pub fn run_static_overlay_service<T: Transport, S: TelemetrySource>(
    lcd: &Lcd<'_, T>,
    image_payload: &[u8],
    telemetry: S,
    settle_delay: Duration,
    overlay: OverlayConfig,
    value_iterations: usize,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    run_display_overlay_service(
        lcd,
        DisplayUpload::image(image_payload.to_vec()),
        telemetry,
        settle_delay,
        overlay,
        value_iterations,
        &mut sleep,
    )
}

pub fn run_display_upload_service<T: Transport>(
    lcd: &Lcd<'_, T>,
    upload: DisplayUpload,
    settle_delay: Duration,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    upload_and_activate_display(lcd, &upload, settle_delay, &mut sleep)
}

pub fn run_display_overlay_service<T: Transport, S: TelemetrySource>(
    lcd: &Lcd<'_, T>,
    upload: DisplayUpload,
    mut telemetry: S,
    settle_delay: Duration,
    overlay: OverlayConfig,
    value_iterations: usize,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    upload_and_activate_display(lcd, &upload, settle_delay, &mut sleep)?;

    logging::info(format!(
        "enabling metric overlay flags 0x{:02x}, interval {}",
        overlay.flags, overlay.interval
    ));
    lcd.set_metric_overlay(overlay.flags, overlay.interval)?;

    logging::info("entering value refresh loop");
    let mut detector = MetricChangeDetector::new(overlay.flags, METRIC_FORCE_RESEND_AFTER);
    for _ in 0..value_iterations {
        let values = telemetry.read()?;
        if detector.observe(values, overlay.update_interval) {
            lcd.set_metric_values(values)?;
        }
        sleep(overlay.update_interval);
    }
    Ok(())
}

fn upload_and_activate_display<T: Transport>(
    lcd: &Lcd<'_, T>,
    upload: &DisplayUpload,
    settle_delay: Duration,
    sleep: &mut impl FnMut(Duration),
) -> io::Result<()> {
    logging::info("opening LCD");
    lcd.open_lcd(true)?;
    if upload.mode == DisplayMode::Gif
        && let Some(reset_payload) = &upload.static_reset_payload
    {
        logging::info(format!(
            "uploading Image reset payload: {} bytes",
            reset_payload.len()
        ));
        lcd.upload_payload_with_options_and_sleeper(
            reset_payload,
            UploadOptions::image(),
            &mut *sleep,
        )?;
        logging::info("setting Image reset mode");
        lcd.apply_mode_cleanly(DisplayMode::Image, &mut *sleep)?;
        logging::info("enabling Image reset template");
        lcd.set_image_template(TemplateKind::Image, true)?;
    }
    if upload.mode == DisplayMode::Gif {
        logging::info("selecting Gif mode before upload");
        lcd.clear_metric_overlay()?;
        lcd.set_mode(DisplayMode::Gif)?;
        lcd.set_image_template(TemplateKind::Gif, true)?;
    }
    logging::info(format!(
        "uploading {:?} payload: {} bytes, {} frames, delay {} ms",
        upload.options.kind,
        upload.payload.len(),
        upload.options.frame_count,
        upload.options.delay_ms
    ));
    lcd.upload_payload_with_options_and_sleeper(&upload.payload, upload.options, &mut *sleep)?;
    logging::info(format!(
        "upload finished; settling for {}s",
        settle_delay.as_secs()
    ));
    sleep(settle_delay);
    logging::info("clearing metric overlay");
    lcd.clear_metric_overlay()?;
    if upload.mode == DisplayMode::Gif {
        logging::info("setting Gif mode");
        lcd.set_mode(DisplayMode::Gif)?;
    } else {
        logging::info(format!("setting {:?} mode", upload.mode));
        lcd.apply_mode_cleanly(upload.mode, &mut *sleep)?;
        if let Some(template_kind) = template_kind_for_mode(upload.mode) {
            logging::info(format!("enabling {:?} image template", template_kind));
            lcd.set_image_template(template_kind, true)?;
        }
    }
    Ok(())
}

pub fn run_static_overlay_loop<T: Transport, S: TelemetrySource>(
    lcd: &Lcd<'_, T>,
    image_payload: &[u8],
    telemetry: S,
    settle_delay: Duration,
    overlay: OverlayConfig,
) -> io::Result<()> {
    run_display_overlay_loop(
        lcd,
        DisplayUpload::image(image_payload.to_vec()),
        telemetry,
        settle_delay,
        overlay,
    )
}

pub fn run_display_overlay_loop<T: Transport, S: TelemetrySource>(
    lcd: &Lcd<'_, T>,
    upload: DisplayUpload,
    telemetry: S,
    settle_delay: Duration,
    overlay: OverlayConfig,
) -> io::Result<()> {
    run_display_overlay_service(
        lcd,
        upload,
        telemetry,
        settle_delay,
        overlay,
        usize::MAX,
        thread::sleep,
    )
}

pub fn run_display_upload_once<T: Transport>(
    lcd: &Lcd<'_, T>,
    upload: DisplayUpload,
    settle_delay: Duration,
) -> io::Result<()> {
    run_display_upload_service(lcd, upload, settle_delay, thread::sleep)
}

fn template_kind_for_mode(mode: DisplayMode) -> Option<TemplateKind> {
    match mode {
        DisplayMode::Image => Some(TemplateKind::Image),
        DisplayMode::Gif => Some(TemplateKind::Gif),
        DisplayMode::ChibiTime => Some(TemplateKind::Pet),
        _ => None,
    }
}
