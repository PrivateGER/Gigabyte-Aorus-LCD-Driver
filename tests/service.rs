use gigabyte_lcd::device::Lcd;
use gigabyte_lcd::protocol::MetricValues;
use gigabyte_lcd::service::{
    DisplayUpload, OverlayConfig, TelemetrySource, run_display_overlay_service,
    run_static_overlay_service,
};
use gigabyte_lcd::transport::Transport;
use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::time::Duration;

#[derive(Default)]
struct RecordingTransport {
    writes: RefCell<Vec<Vec<u8>>>,
}

impl Transport for RecordingTransport {
    fn write(&self, payload: &[u8]) -> io::Result<()> {
        self.writes.borrow_mut().push(payload.to_vec());
        Ok(())
    }
}

struct StaticTelemetry {
    values: MetricValues,
}

impl TelemetrySource for StaticTelemetry {
    fn read(&mut self) -> io::Result<MetricValues> {
        Ok(self.values)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UploadEvent {
    Write(u8),
    Sleep(Duration),
}

struct EventTransport {
    events: Rc<RefCell<Vec<UploadEvent>>>,
}

impl Transport for EventTransport {
    fn write(&self, payload: &[u8]) -> io::Result<()> {
        self.events
            .borrow_mut()
            .push(UploadEvent::Write(payload[0]));
        Ok(())
    }
}

#[test]
fn upload_waits_before_start_command_like_gcc() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let transport = EventTransport {
        events: Rc::clone(&events),
    };
    let lcd = Lcd::new(&transport, 0x21);

    lcd.upload_payload_with_sleeper(
        &[0; 12],
        gigabyte_lcd::protocol::ImageKind::Image,
        |delay| {
            events.borrow_mut().push(UploadEvent::Sleep(delay));
        },
    )
    .unwrap();

    let events = events.borrow();
    assert_eq!(events[0], UploadEvent::Sleep(Duration::from_secs(2)));
    assert_eq!(events[1], UploadEvent::Write(0xf2));
}

#[test]
fn service_sets_overlay_once_then_only_feeds_values() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    let telemetry = StaticTelemetry {
        values: MetricValues {
            temperature_c: 48,
            gpu_usage_percent: 64,
            power_watts: 211,
            ..MetricValues::default()
        },
    };
    let payload = vec![0; 12];

    run_static_overlay_service(
        &lcd,
        &payload,
        telemetry,
        Duration::ZERO,
        OverlayConfig::default(),
        3,
        |_| {},
    )
    .unwrap();

    let writes = transport.writes.borrow();
    let opcodes: Vec<u8> = writes.iter().map(|packet| packet[0]).collect();
    let overlay_selection_count = writes
        .iter()
        .filter(|packet| packet[0] == 0xe1 && packet[5] == 1 && packet[7] == 1 && packet[12] == 1)
        .count();
    let value_count = opcodes.iter().filter(|opcode| **opcode == 0xe3).count();

    assert_eq!(overlay_selection_count, 1);
    assert_eq!(value_count, 3);
    assert!(
        opcodes.iter().position(|opcode| *opcode == 0xe1).unwrap()
            < opcodes.iter().position(|opcode| *opcode == 0xe3).unwrap()
    );
    assert!(
        opcodes.contains(&0xf1),
        "image upload header should be sent"
    );
    assert!(
        writes
            .iter()
            .any(|packet| packet[0] == 0xf1 && packet[9] == 1)
    );
}

#[test]
fn gif_service_uploads_gif_payload_and_switches_to_gif_mode() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    let telemetry = StaticTelemetry {
        values: MetricValues::default(),
    };

    run_display_overlay_service(
        &lcd,
        DisplayUpload::gif(vec![0; 12], 3, 50),
        telemetry,
        Duration::ZERO,
        OverlayConfig::default(),
        1,
        |_| {},
    )
    .unwrap();

    let writes = transport.writes.borrow();
    let header = writes.iter().find(|packet| packet[0] == 0xf1).unwrap();
    let template = writes.iter().find(|packet| packet[0] == 0xea).unwrap();
    let mode = writes
        .iter()
        .find(|packet| packet[0] == 0xe5 && packet[5] == 6)
        .unwrap();

    assert_eq!(header[9], 2);
    assert_eq!(&header[14..16], &3u16.to_be_bytes());
    assert_eq!(header[16], 50);
    assert_eq!(template[5], 1);
    assert_eq!(template[17], 1);
    assert_eq!(mode[0], 0xe5);
}

#[test]
fn gif_service_selects_gif_mode_like_gcc_before_upload() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    let telemetry = StaticTelemetry {
        values: MetricValues::default(),
    };

    run_display_overlay_service(
        &lcd,
        DisplayUpload::gif(vec![0; 12], 3, 50),
        telemetry,
        Duration::ZERO,
        OverlayConfig::default(),
        1,
        |_| {},
    )
    .unwrap();

    let writes = transport.writes.borrow();
    let first_upload_start = writes
        .iter()
        .position(|packet| packet[0] == 0xf2 && packet[5] == 1)
        .unwrap();
    let first_gif_mode = writes
        .iter()
        .position(|packet| packet[0] == 0xe5 && packet[5] == 6)
        .unwrap();
    let first_gif_template = writes
        .iter()
        .position(|packet| packet[0] == 0xea && packet[5] == 1 && packet[17] == 1)
        .unwrap();

    assert!(first_gif_mode < first_upload_start);
    assert!(first_gif_template < first_upload_start);
    assert!(
        writes.iter().all(|packet| packet[0] != 0xf3),
        "GCC mode selection does not send a carousel loop packet for GIF mode"
    );
}

#[test]
fn gif_service_can_upload_static_image_reset_before_real_gif() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    let telemetry = StaticTelemetry {
        values: MetricValues::default(),
    };

    run_display_overlay_service(
        &lcd,
        DisplayUpload::gif(vec![0xaa; 12], 3, 50).with_static_reset(vec![0x55; 12]),
        telemetry,
        Duration::ZERO,
        OverlayConfig::default(),
        1,
        |_| {},
    )
    .unwrap();

    let writes = transport.writes.borrow();
    let headers = writes
        .iter()
        .enumerate()
        .filter(|(_, packet)| packet[0] == 0xf1)
        .collect::<Vec<_>>();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[0].1[9], 1, "reset upload should use image storage");
    assert_eq!(headers[1].1[9], 2, "final upload should use GIF storage");

    let image_mode = writes
        .iter()
        .position(|packet| packet[0] == 0xe5 && packet[5] == 4)
        .unwrap();
    let gif_mode = writes
        .iter()
        .rposition(|packet| packet[0] == 0xe5 && packet[5] == 6)
        .unwrap();
    assert!(headers[0].0 < image_mode);
    assert!(image_mode < headers[1].0);
    assert!(headers[1].0 < gif_mode);
}

#[test]
fn service_uses_selected_metric_overlay_flags() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    let telemetry = StaticTelemetry {
        values: MetricValues::default(),
    };
    let payload = vec![0; 12];
    let flags = (1 << 1) | (1 << 3) | (1 << 6);

    run_static_overlay_service(
        &lcd,
        &payload,
        telemetry,
        Duration::ZERO,
        OverlayConfig { flags, interval: 7 },
        1,
        |_| {},
    )
    .unwrap();

    let writes = transport.writes.borrow();
    let overlay = writes
        .iter()
        .find(|packet| packet[0] == 0xe1 && packet[6] == 1 && packet[8] == 1 && packet[11] == 1)
        .unwrap();

    assert_eq!(&overlay[5..13], &[0, 1, 0, 1, 0, 0, 1, 0]);
    assert_eq!(overlay[13], 7);
}
