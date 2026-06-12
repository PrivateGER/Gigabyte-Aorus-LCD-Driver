//! Hardware integration test for the NVIDIA RM API transport.
//!
//! Requires a Gigabyte LCD panel on an NVIDIA GPU i2c bus and access to
//! /dev/nvidiactl, so it is ignored by default:
//!
//! ```text
//! cargo test --test rmapi_hw -- --ignored --nocapture
//! ```

use gigabyte_lcd::protocol::{DEFAULT_ADDR, DEFAULT_BUS, I2C_PAGE_SIZE};
use gigabyte_lcd::rmapi::{I2cSpeed, NvRmI2cTransport};
use gigabyte_lcd::transport::Transport;
use std::time::Instant;

const FW_VERSION_QUERY: [u8; 5] = [0xd6, 0xcb, 0x55, 0xac, 0x38];

#[test]
#[ignore = "needs LCD hardware and /dev/nvidiactl"]
fn firmware_query_works_and_is_faster_than_100khz_i2cdev() {
    let transport = NvRmI2cTransport::open(DEFAULT_BUS, DEFAULT_ADDR, I2cSpeed::Khz400)
        .expect("RM transport should open on LCD hardware");

    let mut page = vec![0u8; I2C_PAGE_SIZE];
    page[..FW_VERSION_QUERY.len()].copy_from_slice(&FW_VERSION_QUERY);

    let started = Instant::now();
    let response = transport
        .write_read(&page, 4)
        .expect("firmware version query should succeed");
    let elapsed = started.elapsed();

    println!("firmware response: {response:02x?}, transfer took {elapsed:?}");
    assert_eq!(response[0], 0xd6, "panel should echo the query opcode");
    assert!(
        elapsed.as_millis() < 20,
        "400 kHz transfer should beat the ~27 ms i2c-dev baseline, took {elapsed:?}"
    );
}
