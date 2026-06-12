use gigabyte_lcd::device::Lcd;
use gigabyte_lcd::protocol::MetricValues;
use gigabyte_lcd::service::{
    DisplayUpload, MetricChangeDetector, OverlayConfig, TelemetrySource,
    run_display_overlay_service, run_display_upload_service, run_static_overlay_service,
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

struct SequenceTelemetry {
    values: Vec<MetricValues>,
    index: usize,
}

impl SequenceTelemetry {
    fn new(values: Vec<MetricValues>) -> Self {
        Self { values, index: 0 }
    }
}

impl TelemetrySource for SequenceTelemetry {
    fn read(&mut self) -> io::Result<MetricValues> {
        let values = self.values[self.index.min(self.values.len() - 1)];
        self.index += 1;
        Ok(values)
    }
}

fn count_value_packets(writes: &[Vec<u8>]) -> usize {
    writes.iter().filter(|packet| packet[0] == 0xe3).count()
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
fn service_sets_overlay_once_then_feeds_values_only_when_they_change() {
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

    assert_eq!(overlay_selection_count, 1);
    assert_eq!(
        count_value_packets(&writes),
        1,
        "identical metric samples should be written once, not re-sent every tick"
    );
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
fn service_resends_values_when_a_displayed_metric_changes() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    // 48 -> 49 is one-degree boundary flapping (skipped); 50 is 2 degrees
    // from the last written sample and must be sent.
    let telemetry = SequenceTelemetry::new(vec![
        MetricValues {
            temperature_c: 48,
            ..MetricValues::default()
        },
        MetricValues {
            temperature_c: 49,
            ..MetricValues::default()
        },
        MetricValues {
            temperature_c: 50,
            ..MetricValues::default()
        },
    ]);

    run_static_overlay_service(
        &lcd,
        &[0; 12],
        telemetry,
        Duration::ZERO,
        OverlayConfig::default(),
        3,
        |_| {},
    )
    .unwrap();

    assert_eq!(count_value_packets(&transport.writes.borrow()), 2);
}

#[test]
fn service_ignores_jitter_on_metrics_the_overlay_does_not_display() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    // Default overlay flags show temp/usage/power; clock and fan jitter must
    // not cause panel writes.
    let telemetry = SequenceTelemetry::new(vec![
        MetricValues {
            gpu_clock_mhz: 2400,
            fan_rpm: 1200,
            ..MetricValues::default()
        },
        MetricValues {
            gpu_clock_mhz: 2750,
            fan_rpm: 1480,
            ..MetricValues::default()
        },
        MetricValues {
            gpu_clock_mhz: 2100,
            fan_rpm: 900,
            ..MetricValues::default()
        },
    ]);

    run_static_overlay_service(
        &lcd,
        &[0; 12],
        telemetry,
        Duration::ZERO,
        OverlayConfig::default(),
        3,
        |_| {},
    )
    .unwrap();

    assert_eq!(count_value_packets(&transport.writes.borrow()), 1);
}

#[test]
fn service_skips_sub_threshold_power_jitter_but_sends_accumulated_drift() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    // 211 -> 212 is below the 3 W threshold against the last *sent* value;
    // 214 is 3 W away from 211 and must be written even though each single
    // step was small.
    let telemetry = SequenceTelemetry::new(vec![
        MetricValues {
            power_watts: 211,
            ..MetricValues::default()
        },
        MetricValues {
            power_watts: 212,
            ..MetricValues::default()
        },
        MetricValues {
            power_watts: 214,
            ..MetricValues::default()
        },
    ]);

    run_static_overlay_service(
        &lcd,
        &[0; 12],
        telemetry,
        Duration::ZERO,
        OverlayConfig::default(),
        3,
        |_| {},
    )
    .unwrap();

    assert_eq!(count_value_packets(&transport.writes.borrow()), 2);
}

#[test]
fn service_force_resends_stable_values_after_the_staleness_cap() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);
    let telemetry = StaticTelemetry {
        values: MetricValues {
            temperature_c: 48,
            ..MetricValues::default()
        },
    };

    // 20 s ticks: send at tick 1, skip at +20 s, force-resend at +40 s (>= 30 s cap).
    run_static_overlay_service(
        &lcd,
        &[0; 12],
        telemetry,
        Duration::ZERO,
        OverlayConfig {
            update_interval: Duration::from_secs(20),
            ..OverlayConfig::default()
        },
        3,
        |_| {},
    )
    .unwrap();

    assert_eq!(count_value_packets(&transport.writes.borrow()), 2);
}

#[test]
fn upload_service_sets_image_and_exits_without_metric_monitoring() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);

    run_display_upload_service(
        &lcd,
        DisplayUpload::image(vec![0; 12]),
        Duration::ZERO,
        |_| {},
    )
    .unwrap();

    let writes = transport.writes.borrow();
    assert!(
        writes
            .iter()
            .any(|packet| packet[0] == 0xf1 && packet[9] == 1),
        "image upload header should be sent"
    );
    assert!(
        writes
            .iter()
            .any(|packet| packet[0] == 0xe5 && packet[5] == 4),
        "image mode should be selected"
    );
    assert!(
        writes.iter().all(|packet| packet[0] != 0xe3),
        "metric values should not be sent"
    );
    assert!(
        writes
            .iter()
            .filter(|packet| packet[0] == 0xe1)
            .all(|packet| packet[5..13].iter().all(|value| *value == 0)),
        "metric overlay should only be cleared, never enabled"
    );
}

#[test]
fn upload_service_sets_gif_and_exits_without_metric_monitoring() {
    let transport = RecordingTransport::default();
    let lcd = Lcd::new(&transport, 0x21);

    run_display_upload_service(
        &lcd,
        DisplayUpload::gif(vec![0; 12], 3, 50),
        Duration::ZERO,
        |_| {},
    )
    .unwrap();

    let writes = transport.writes.borrow();
    let header = writes.iter().find(|packet| packet[0] == 0xf1).unwrap();
    assert_eq!(header[9], 2);
    assert_eq!(&header[14..16], &3u16.to_be_bytes());
    assert_eq!(header[16], 50);
    assert!(
        writes
            .iter()
            .any(|packet| packet[0] == 0xe5 && packet[5] == 6),
        "gif mode should be selected"
    );
    assert!(
        writes.iter().all(|packet| packet[0] != 0xe3),
        "metric values should not be sent"
    );
    assert!(
        writes
            .iter()
            .filter(|packet| packet[0] == 0xe1)
            .all(|packet| packet[5..13].iter().all(|value| *value == 0)),
        "metric overlay should only be cleared, never enabled"
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
fn detector_sends_first_sample_even_with_no_displayed_metrics() {
    let mut detector = MetricChangeDetector::new(0, Duration::from_secs(30));

    assert!(detector.observe(MetricValues::default(), Duration::from_secs(1)));
    assert!(
        !detector.observe(
            MetricValues {
                temperature_c: 99,
                power_watts: 500,
                ..MetricValues::default()
            },
            Duration::from_secs(1),
        ),
        "with no metrics displayed, value changes must not trigger writes"
    );
}

#[test]
fn detector_treats_thresholds_as_inclusive_and_handles_wraparound_diffs() {
    let flags = u8::MAX;
    let mut detector = MetricChangeDetector::new(flags, Duration::from_secs(3600));
    assert!(detector.observe(
        MetricValues {
            gpu_usage_percent: 100,
            ..MetricValues::default()
        },
        Duration::ZERO,
    ));

    // 1 pp below threshold: skip; drop from 100 to 0 (adversarial direction): send.
    assert!(!detector.observe(
        MetricValues {
            gpu_usage_percent: 99,
            ..MetricValues::default()
        },
        Duration::ZERO,
    ));
    assert!(detector.observe(MetricValues::default(), Duration::ZERO));

    // Exactly at the 2 pp threshold (0 -> 2) must send: >= is inclusive.
    assert!(detector.observe(
        MetricValues {
            gpu_usage_percent: 2,
            ..MetricValues::default()
        },
        Duration::ZERO,
    ));
}

#[test]
fn detector_force_resend_fires_at_exactly_the_staleness_cap() {
    let cap = Duration::from_secs(30);
    let mut detector = MetricChangeDetector::new(0, cap);
    assert!(detector.observe(MetricValues::default(), Duration::ZERO));

    // One second short of the cap: skip; reaching exactly the cap: send.
    assert!(!detector.observe(MetricValues::default(), cap - Duration::from_secs(1)));
    assert!(detector.observe(MetricValues::default(), Duration::from_secs(1)));
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
        OverlayConfig {
            flags,
            interval: 7,
            ..OverlayConfig::default()
        },
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
