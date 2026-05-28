# Gigabyte GPU LCD Protocol Specification

## 1. Scope

This contains the reverse-engineered control protocol for Gigabyte GPU LCD panels using the older Gigabyte Control Center LCD command format, extracted from a decompiled Gigabyte Control Center install.

The implementation targets this device profile specifically:

| Field | Value |
| --- | --- |
| GPU | Gigabyte GeForce RTX 5080 AORUS Master ICE 16GB |
| LCD size | 320 x 170 pixels |
| Linux I2C device | `/dev/i2c-1` |
| 7-bit I2C address | `0x61` |
| GCC write address equivalent | `0xc2` |
| GCC device id | `0x21` |
| Firmware readback | `F1.4` |

Other Gigabyte LCD panels/GPUs may share the protocol but use different I2C buses,
addresses, device ids, storage offsets, or timing requirements.

## 2. Terminology

Command packets are host-to-LCD I2C writes. Readback commands use an I2C
write-read transaction. Multi-byte integer byte order is specified per field.

## 3. Transport

### 3.1 I2C Framing

Command packets MUST be exactly 256 bytes.

All old-protocol command packets use this format:

```text
u8 opcode
u8[4] magic = cb 55 ac 38
u8[] args
u8[] zero_padding_to_256_bytes
```

Upload data pages are raw 256-byte writes and do not include the magic prefix.

### 3.2 Readback Limits

The protocol exposes firmware and mode readback. It does not expose a readback for image decode completion,
GIF decode completion, current visible frame, or panel Loading state.

## 4. Display Modes

| Mode id | Name | `0xe5` argument |
| --- | --- | --- |
| `0` | Faith1 | `1` |
| `1` | Faith2 | `2` |
| `2` | Faith3 | `3` |
| `3` | Static image | `4` |
| `4` | Vendor text slot | `5` |
| `5` | GIF/animation | `6` |
| `6` | Chibi/time | `7` |
| `7` | Carousel | `10` |

For all modes except carousel, the `0xe5` argument is `mode id + 1`. Carousel
uses wire mode `9`, therefore its command argument is `10`.

## 5. Commands

### 5.1 Command Summary

| Opcode | Name | Direction | Purpose                                                         |
| --- | --- | --- |-----------------------------------------------------------------|
| `0xd6` | Read firmware | write-read | Diagnostic read.                                                |
| `0xde` | Read mode | write-read | Diagnostic read of controller mode/enabled state.               |
| `0xe7` | Open LCD | write | Enables or disables the LCD controller.                         |
| `0xe5` | Set mode | write | Selects display mode.                                           |
| `0xe1` | Set metric overlay | write | Selects overlay fields and rotation interval.                   |
| `0xe3` | Set metric values | write | Refreshes cached metric values.                                 |
| `0xea` | Set image template | write | Configures template/color/positions for image-like modes.       |
| `0xf1` | Upload header | write | Declares payload target, pages, frame count, delay, chunk mode. |
| `0xf2 0x01` | Upload start | write | Starts an upload transaction.                                   |
| `0xf2 0x02` | Upload finish | write | Finishes an upload transaction.                                 |
| `0xf3` | Set loop list | write | Configures carousel/loop mode list.                             |
| `0xaa` | Save | write | Persists settings. Not used by default here.                    |

### 5.2 Open LCD: `0xe7`

Arguments:

| Byte | Meaning |
| --- | --- |
| `0x01` | Enable LCD |
| `0x02` | Disable LCD |

The service MUST enable the LCD before upload or mode setup.

### 5.3 Set Mode: `0xe5`

Arguments:

```text
u8 mode_argument
```

`mode_argument` is defined by the table in section 4.

### 5.4 Set Loop List: `0xf3`

Arguments:

```text
u8 interval
u8[] mode_arguments_without_carousel_special_case
```

The static service uses this command to lock the loop to image mode before
entering steady state.

### 5.5 Set Image Template: `0xea`

Arguments:

```text
u8 template_kind
u8 red
u8 green
u8 blue
u16 image_x_be
u16 image_y_be
u16 data_x_be
u16 data_y_be
u8 enabled
```

Template kinds:

| Value | Kind |
| --- | --- |
| `1` | GIF |
| `2` | Image |
| `3` | Pet/chibi |

Default implementation values:

```text
color: ff ff ff
image position: 0, 320
data position: 146, 64
enabled: 1
```

### 5.6 Set Metric Overlay: `0xe1`

Arguments:

```text
u8 gpu_temperature_enabled
u8 gpu_clock_enabled
u8 gpu_usage_enabled
u8 fan_speed_enabled
u8 vram_clock_enabled
u8 vram_usage_enabled
u8 fps_enabled
u8 power_tgp_enabled
u8 rotation_interval
```

Flag bit mapping used by the CLI:

| Bit | Metric |
| --- | --- |
| `0` | GPU temperature |
| `1` | GPU clock |
| `2` | GPU usage |
| `3` | Fan speed |
| `4` | VRAM clock |
| `5` | VRAM usage |
| `6` | FPS |
| `7` | TGP/power |

The release service selects:

```text
flags: 0x85
fields: GPU temperature, GPU usage, TGP/power
rotation interval: 4
```

`0xe1` is a setup command. You should NOT send it repeatedly, it has intensive performance issues.

### 5.7 Set Metric Values: `0xe3`

Arguments:

```text
u8 gpu_temperature_c
u16 gpu_clock_mhz_be
u8 gpu_usage_percent
u16 fan_rpm_be
u16 vram_clock_mhz_be
u8 vram_usage_percent
u16 fps_be
u16 power_watts_be
```

The LCD displays cached values from this packet. It does not poll GPU telemetry
on its own!

The release service sends `0xe3` once per second in steady state.

## 6. Upload Protocol

### 6.1 Upload Transaction

An upload transaction MUST use this sequence:

```text
sleep 2 seconds
send 0xf2 0x01
sleep 500 ms
send 0xf1 upload header
perform chunk preparation sleeps
sleep 1 second
send 256-byte payload pages, sleeping 1 ms after each page
sleep 500 ms
send 0xf2 0x02
```

### 6.2 Upload Header: `0xf1`

Arguments:

```text
u32 target_address_be
u8 storage_type
u32 page_count_be
u16 frame_count_be
u8 frame_delay_ms
u8 chunk_mode
u8 reserved_zero
```

Page count is:

```text
payload_size / 256 + 1
```

The extra page is included even when `payload_size` is an exact multiple of
256.

### 6.3 Chunk Modes

| Payload size | Chunk mode | Preparation unit | Delay per unit |
| --- | --- | --- | --- |
| `< 20480 bytes` | `1` | 4096 bytes | 400 ms |
| `>= 20480 bytes` | `2` | 65536 bytes | 2 s |

### 6.4 Storage Targets

For 50-series device ids `0x18`, `0x19`, `0x20`, `0x21`, and `0x22`:

| Payload kind | Target address | Storage type | Display mode |
| --- | --- | --- | --- |
| Static image | `0x01300000` | `1` | `3` |
| Text | `0x01320000` | `1` | `4` |
| GIF | `0x00000000` | `2` | `5` |

For older/non-50-series ids, static image uses `0x01f26000` and text uses
`0x01f00000`.

## 7. Payload Formats

### 7.1 Pixel Format

Image pixels are RGB565 little-endian.

Panel dimensions are:

```text
width: 320
height: 170
```

### 7.2 Static Image Container

Static image payload:

```text
u16_le frame_count = 1
u32_le frame_0_end_offset_minus_one
u16_le width
u16_le height
u16_le image_format = 1
u8[] rgb565_le_pixels
```

For a full 320 x 170 image, payload size is 108812 bytes.

### 7.3 Animation Container

Animation/GIF payload:

```text
u16_le frame_count
frame_header[frame_count]
u8[] rle_frame_streams
```

Each frame header is:

```text
u32_le frame_end_offset_minus_one
u16_le width
u16_le height
u16_le image_format = 3
```

Frame streams are RGB565 RLE streams.

### 7.4 RLE Stream

RLE is encoded as 16-bit little-endian words.

Literal block:

```text
u16_le count, high bit clear
u16_le[count] literal_rgb565_values
```

Repeat block:

```text
u16_le count_with_high_bit_set
u16_le repeated_rgb565_value
```

The repeat count is `count_with_high_bit_set & 0x7fff`. Literal and repeat
blocks MUST NOT encode more than 32767 pixels.

## 8. Supported States

### 8.1 Static Service State

This is the GCC stable release state machine:

```text
Disabled/unknown
  -> enable LCD
  -> upload static image payload
  -> wait 20 seconds
  -> clear metric overlay
  -> lock loop to image mode
  -> set image mode
  -> set image mode again after 300 ms
  -> enable image template
  -> set metric overlay flags 0x85 interval 4
  -> steady metric refresh
```

In steady metric refresh, the service MUST send only `0xe3` value packets at
the configured telemetry interval.

The service MUST NOT perform image uploads, mode changes, template writes,
save commands, or overlay-selection writes from the steady-state loop.

### 8.2 Experimental GIF State

GIF mode is supported for manual testing only. It is not the release service
default.

The current safe GIF startup profile is:

```text
upload static reset image first
apply image mode/template after reset
set GIF mode before GIF upload
enable GIF template before GIF upload
normalized content height <= 150 px
frame delay >= 100 ms
frame budget <= 24 source frames
post-GIF-upload delay = 5 seconds
```

The tested GIF payload was:

```text
62410 bytes
20 frames
100 ms frame delay
```

The GIF state machine is:

```text
Disabled/unknown
  -> enable LCD
  -> upload static reset image
  -> apply image mode/template
  -> clear metric overlay
  -> set GIF mode
  -> enable GIF template
  -> upload GIF payload
  -> wait 5 seconds
  -> clear metric overlay
  -> set GIF mode
  -> set metric overlay flags 0x85 interval 4
  -> steady metric refresh
```

GIF decode success cannot be verified automatically through known readbacks.

### 8.3 Recovery State

If the panel is white, stuck on Loading, or visibly refreshing incorrectly,
recover through the static service state:

```text
stop experimental/test service
start static image service
allow 20 second static settle
enter steady metric refresh
```

## 9. Performance Requirements

The host SHOULD use in-process NVML for telemetry. It SHOULD NOT invoke
`nvidia-smi` once per refresh.

The steady-state refresh loop MUST avoid repeated setup commands. In testing,
repeated `0xe1` overlay-selection writes and repeated mode/setup writes caused
visible frame-time hitches while games were running.

All LCD I2C writes SHOULD be serialized through a single writer.

## 10. Ex Protocol Status

Gigabyte Control Center also contains a newer Ex protocol path. It is not
implemented by this project.

Known Ex details:

```text
candidate 7-bit address: 0x76
candidate shifted address: 0xec
command prefix shape: <opcode> 0x01 ...
candidate Linux bus that ACKed 0x76: /dev/i2c-4
```

Known Ex commands from recovered Windows code:

| Opcode | Meaning |
| --- | --- |
| `0x10` | Firmware/version read |
| `0x15` | Open LCD |
| `0x16` | Set mode |
| `0x17` | Set display overlay |
| `0x18` | Overlay position |
| `0x23` | Metric values |

Linux tests against the Ex candidate returned zero firmware data and produced
no visible LCD effect. Ex MUST be treated as unsupported until a working Linux
transport is proven.

## 11. Windows Transport Notes

The Windows path uses `AorusLcdService.exe`, `ucVga.dll`, and `GvDisplayA.dll`.
The native layer dispatches I2C operations through private NVIDIA driver entry
points rather than Linux `/dev/i2c-*`.

Recovered private NVAPI QueryInterface ids:

```text
0x283AC65A -> NvAPI_I2CWriteEx
0x4D7B0709 -> NvAPI_I2CReadEx
```

The Windows behavior relevant to this implementation is operational: perform
setup writes separately, then refresh only metric values on the hot path.
