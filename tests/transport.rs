use gigabyte_lcd::transport::{LinuxI2cTransport, Transport};
use std::io;

#[test]
fn write_read_rejects_write_payloads_larger_than_one_i2c_page_before_opening_device() {
    let transport = LinuxI2cTransport::new(250, 0x61);

    let error = transport.write_read(&[0; 257], 1).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("write payload"));
}

#[test]
fn write_read_rejects_reads_larger_than_kernel_i2c_message_length_before_opening_device() {
    let transport = LinuxI2cTransport::new(250, 0x61);

    let error = transport
        .write_read(&[0; 1], u16::MAX as usize + 1)
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("read length"));
}
