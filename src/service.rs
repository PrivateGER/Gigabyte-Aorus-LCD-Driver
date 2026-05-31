use crate::device::{Lcd, UploadOptions};
use crate::logging;
use crate::protocol::{DisplayMode, MetricValues, TemplateKind};
use crate::transport::Transport;
use std::io;
use std::thread;
use std::time::Duration;

pub const DEFAULT_LCD_INTERVAL_OVERLAY_FLAGS: u8 = (1 << 0) | (1 << 2) | (1 << 7);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayConfig {
    pub flags: u8,
    pub interval: u8,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            flags: DEFAULT_LCD_INTERVAL_OVERLAY_FLAGS,
            interval: 4,
        }
    }
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
    for _ in 0..value_iterations {
        lcd.set_metric_values(telemetry.read()?)?;
        sleep(Duration::from_secs(1));
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
