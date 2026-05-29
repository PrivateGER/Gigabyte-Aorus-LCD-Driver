mod args;

use args::{Args, ProbeRgbArgs, RunArgs};
use gigabyte_lcd::device::Lcd;
use gigabyte_lcd::gif::gif_payload_from_path;
use gigabyte_lcd::image::{mascot_background_from_path, single_frame_payload};
use gigabyte_lcd::logging;
use gigabyte_lcd::rgb_discovery::run_rgb_discovery;
use gigabyte_lcd::service::{DisplayUpload, run_display_overlay_loop};
use gigabyte_lcd::telemetry::NvmlTelemetry;
use gigabyte_lcd::transport::LinuxI2cTransport;
use std::io;

fn main() -> io::Result<()> {
    let args = Args::parse_cli()?;
    logging::set_level(args.log_level());
    match args {
        Args::Run(args) => run_lcd_service(args),
        Args::ProbeRgb(args) => run_rgb_probe(args),
    }
}

fn run_lcd_service(args: RunArgs) -> io::Result<()> {
    let upload = if let Some(gif_path) = &args.gif {
        let gif = gif_payload_from_path(gif_path, &args.gif_limits)?;
        let reset_image = mascot_background_from_path(&args.mascot)?;
        DisplayUpload::gif(gif.payload, gif.frames.len() as u16, gif.upload_delay_ms)
            .with_static_reset(single_frame_payload(&reset_image)?)
    } else {
        let image = mascot_background_from_path(&args.mascot)?;
        DisplayUpload::image(single_frame_payload(&image)?)
    };
    let transport = LinuxI2cTransport::new(args.bus, args.addr);
    let lcd = Lcd::new(&transport, args.device_id);
    let telemetry = NvmlTelemetry::open(args.gpu_index)?;

    run_display_overlay_loop(
        &lcd,
        upload,
        telemetry,
        args.image_settle_delay,
        args.overlay,
    )
}

fn run_rgb_probe(args: ProbeRgbArgs) -> io::Result<()> {
    let transport = LinuxI2cTransport::with_path(args.i2c_dev, 0);
    let report = run_rgb_discovery(&transport, args.addresses);
    println!("{report}");
    if report.any_success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no RGB backend responded",
        ))
    }
}
