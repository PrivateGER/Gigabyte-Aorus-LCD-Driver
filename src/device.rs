use crate::protocol::{
    DisplayMode, ImageKind, MetricValues, TemplateKind, build_image_template_packet,
    build_loop_packet, build_metric_overlay_packet, build_metric_values_packet, build_open_packet,
    build_save_packet, build_set_mode_packet, build_upload_finish_packet,
    build_upload_header_packet, build_upload_start_packet, page_count_for_gcc, upload_chunk_mode,
};
use crate::transport::Transport;
use std::io;
use std::time::Duration;

pub struct Lcd<'a, T: Transport> {
    transport: &'a T,
    device_led_id: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadOptions {
    pub kind: ImageKind,
    pub frame_count: u16,
    pub delay_ms: u8,
}

impl UploadOptions {
    pub fn image() -> Self {
        Self {
            kind: ImageKind::Image,
            frame_count: 0,
            delay_ms: 0,
        }
    }

    pub fn gif(frame_count: u16, delay_ms: u8) -> Self {
        Self {
            kind: ImageKind::Gif,
            frame_count,
            delay_ms,
        }
    }
}

impl<'a, T: Transport> Lcd<'a, T> {
    pub fn new(transport: &'a T, device_led_id: u8) -> Self {
        Self {
            transport,
            device_led_id,
        }
    }

    pub fn open_lcd(&self, enabled: bool) -> io::Result<()> {
        self.transport.write(&build_open_packet(enabled))
    }

    pub fn set_mode(&self, mode: DisplayMode) -> io::Result<()> {
        self.transport.write(&build_set_mode_packet(mode))
    }

    pub fn save(&self) -> io::Result<()> {
        self.transport.write(&build_save_packet())
    }

    pub fn clear_metric_overlay(&self) -> io::Result<()> {
        self.transport.write(&build_metric_overlay_packet(0, 1))
    }

    pub fn set_metric_overlay(&self, flags: u8, interval: u8) -> io::Result<()> {
        self.transport
            .write(&build_metric_overlay_packet(flags, interval))
    }

    pub fn set_image_template(&self, kind: TemplateKind, enabled: bool) -> io::Result<()> {
        self.transport
            .write(&build_image_template_packet(kind, enabled))
    }

    pub fn set_metric_values(&self, values: MetricValues) -> io::Result<()> {
        self.transport.write(&build_metric_values_packet(values))
    }

    pub fn set_loop(&self, modes: &[DisplayMode], interval: u8) -> io::Result<()> {
        self.transport.write(&build_loop_packet(modes, interval))
    }

    pub fn upload_payload_with_sleeper(
        &self,
        payload: &[u8],
        kind: ImageKind,
        mut sleep: impl FnMut(Duration),
    ) -> io::Result<()> {
        self.upload_payload_with_options_and_sleeper(
            payload,
            UploadOptions {
                kind,
                frame_count: 0,
                delay_ms: 0,
            },
            &mut sleep,
        )
    }

    pub fn upload_payload_with_options_and_sleeper(
        &self,
        payload: &[u8],
        options: UploadOptions,
        mut sleep: impl FnMut(Duration),
    ) -> io::Result<()> {
        sleep(Duration::from_secs(2));
        self.transport.write(&build_upload_start_packet())?;
        sleep(Duration::from_millis(500));
        self.transport.write(&build_upload_header_packet(
            payload.len(),
            options.kind,
            self.device_led_id,
            options.frame_count,
            options.delay_ms,
        ))?;

        let (_, chunk_size, prep_delay) = upload_chunk_mode(payload.len());
        let prep_chunks = payload.len().div_ceil(chunk_size);
        for _ in 0..prep_chunks {
            sleep(prep_delay);
        }

        sleep(Duration::from_secs(1));
        for page in 0..page_count_for_gcc(payload.len()) as usize {
            let start = page * 256;
            let mut block = vec![0; 256];
            if start < payload.len() {
                let end = (start + 256).min(payload.len());
                block[..end - start].copy_from_slice(&payload[start..end]);
            }
            self.transport.write(&block)?;
            sleep(Duration::from_millis(1));
        }

        sleep(Duration::from_millis(500));
        self.transport.write(&build_upload_finish_packet())
    }

    pub fn apply_image_mode_cleanly(&self, mut sleep: impl FnMut(Duration)) -> io::Result<()> {
        self.clear_metric_overlay()?;
        self.apply_mode_cleanly(DisplayMode::Image, &mut sleep)
    }

    pub fn apply_mode_cleanly(
        &self,
        mode: DisplayMode,
        mut sleep: impl FnMut(Duration),
    ) -> io::Result<()> {
        self.set_loop(&[mode], 1)?;
        self.set_mode(mode)?;
        sleep(Duration::from_millis(300));
        self.set_mode(mode)
    }
}
